//! Local Team Create script drafts. Files are private and replaced atomically.
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Draft {
    pub base: String,
    pub text: String,
    #[serde(default)]
    pub submitted: bool,
    #[serde(default)]
    pub rejected: bool,
}

pub struct DraftStore {
    root: PathBuf,
    keys: HashMap<u64, String>,
    sources: HashMap<u64, String>,
    drafts: HashMap<u64, Draft>,
    dirty: HashSet<u64>,
    flushed: Instant,
}

impl DraftStore {
    pub fn for_place(place: u64) -> Self {
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(std::env::temp_dir)
                    .join(".local/state")
            });
        Self::new(base.join("rustudio/script-drafts").join(place.to_string()))
    }
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            keys: HashMap::new(),
            sources: HashMap::new(),
            drafts: HashMap::new(),
            dirty: HashSet::new(),
            flushed: Instant::now(),
        }
    }
    pub fn register(&mut self, id: u64, guid: Option<&str>, source: String) -> Result<(), String> {
        if self.keys.contains_key(&id) {
            self.receive(id, source);
            return Ok(());
        }
        let key = guid
            .map(|g| {
                g.chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .take(128)
                    .collect::<String>()
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("ref-{id}"));
        self.keys.insert(id, key);
        self.sources.insert(id, source.clone());
        let path = self.path(id).unwrap();
        match std::fs::read(&path) {
            Ok(bytes) => {
                let mut draft: Draft = serde_json::from_slice(&bytes).map_err(|_| {
                    "A saved script draft could not be read; its recovery file was preserved"
                })?;
                if draft.text == source {
                    self.dirty.insert(id);
                } else {
                    // A sent draft can still need recovery after a disconnect.
                    draft.submitted = false;
                    self.drafts.insert(id, draft);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("A saved script draft could not be opened".into()),
        }
        Ok(())
    }
    pub fn get(&self, id: u64) -> Option<&Draft> {
        self.drafts.get(&id)
    }
    pub fn source(&self, id: u64) -> Option<&str> {
        self.sources.get(&id).map(String::as_str)
    }
    pub fn conflict(&self, id: u64) -> bool {
        self.drafts.get(&id).is_some_and(|d| {
            d.rejected
                || self
                    .sources
                    .get(&id)
                    .is_some_and(|s| s != &d.base && s != &d.text)
        })
    }
    pub fn has_unsubmitted(&self) -> bool {
        self.drafts.values().any(|d| !d.submitted)
    }
    pub fn edit(&mut self, id: u64, text: String) -> Result<(), String> {
        let base = self
            .sources
            .get(&id)
            .ok_or("The server's script source is not available yet")?
            .clone();
        if text == base {
            self.drafts.remove(&id);
        } else {
            let draft = self.drafts.entry(id).or_insert_with(|| Draft {
                base: base.clone(),
                text: String::new(),
                submitted: false,
                rejected: false,
            });
            if draft.submitted {
                draft.base = base;
            }
            draft.text = text;
            draft.submitted = false;
            draft.rejected = false;
        }
        self.dirty.insert(id);
        Ok(())
    }
    /// Returns the text to send; the caller marks it submitted only after queueing.
    pub fn commit_text(&self, id: u64, overwrite: bool) -> Result<String, String> {
        if self.conflict(id) && !overwrite {
            return Err(
                "The server changed this script. Review the difference before replacing it".into(),
            );
        }
        self.drafts
            .get(&id)
            .filter(|d| !d.submitted)
            .map(|d| d.text.clone())
            .ok_or_else(|| "There is no uncommitted draft".into())
    }
    pub fn rejected(&mut self, id: u64) {
        if let Some(d) = self.drafts.get_mut(&id) {
            d.rejected = true;
            self.dirty.insert(id);
        }
    }
    pub fn confirmed(&mut self, id: u64, text: String) {
        // The editor can change again while an atomic commit is in flight.
        // Only acknowledge the submitted snapshot, keeping subsequent typing.
        if let Some(draft) = self.drafts.get_mut(&id) {
            if draft.text != text {
                draft.base = text.clone();
                draft.rejected = false;
                self.dirty.insert(id);
            }
        }
        self.receive(id, text);
    }
    pub fn submitted(&mut self, id: u64) {
        if let Some(d) = self.drafts.get_mut(&id) {
            d.submitted = true;
            self.sources.insert(id, d.text.clone());
            self.dirty.insert(id);
        }
    }
    pub fn receive(&mut self, id: u64, source: String) {
        if self.drafts.get(&id).is_some_and(|d| d.text == source) {
            self.drafts.remove(&id);
            self.dirty.insert(id);
        } else if self.drafts.get(&id).is_some_and(|d| d.submitted) {
            // A newer peer edit supersedes an already submitted draft.
            self.drafts.remove(&id);
            self.dirty.insert(id);
        }
        self.sources.insert(id, source);
    }
    pub fn discard(&mut self, id: u64) -> Option<String> {
        self.drafts.remove(&id);
        self.dirty.insert(id);
        self.sources.get(&id).cloned()
    }
    fn path(&self, id: u64) -> Option<PathBuf> {
        self.keys
            .get(&id)
            .map(|key| self.root.join(format!("{key}.json")))
    }
    pub fn flush_due(&mut self) -> Result<(), String> {
        if self.flushed.elapsed() >= Duration::from_millis(500) {
            self.flushed = Instant::now();
            self.flush()
        } else {
            Ok(())
        }
    }
    pub fn flush(&mut self) -> Result<(), String> {
        if self.dirty.is_empty() {
            return Ok(());
        }
        use std::{
            io::Write,
            os::unix::fs::{DirBuilderExt, OpenOptionsExt},
        };
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder
            .create(&self.root)
            .map_err(|_| "Could not create the private script draft folder")?;
        for id in self.dirty.iter().copied().collect::<Vec<_>>() {
            let Some(path) = self.path(id) else {
                continue;
            };
            if let Some(draft) = self.drafts.get(&id) {
                let temp = path.with_extension("json.tmp");
                let data =
                    serde_json::to_vec(draft).map_err(|_| "Could not encode a script draft")?;
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .truncate(true)
                    .write(true)
                    .mode(0o600)
                    .open(&temp)
                    .map_err(|_| "Could not write a script draft")?;
                file.write_all(&data)
                    .and_then(|_| file.sync_all())
                    .map_err(|_| "Could not finish writing a script draft")?;
                std::fs::rename(temp, path)
                    .map_err(|_| "Could not replace a script draft recovery file")?;
            } else {
                match std::fs::remove_file(path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err("Could not remove the script draft recovery file".into()),
                }
            }
            self.dirty.remove(&id);
        }
        self.flushed = Instant::now();
        Ok(())
    }
}
impl Drop for DraftStore {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commit_acknowledges_only_the_submitted_snapshot() {
        let root =
            std::env::temp_dir().join(format!("rustudio-draft-inflight-{}", std::process::id()));
        let mut d = DraftStore::new(root.clone());
        d.register(1, Some("inflight"), "base".into()).unwrap();
        d.edit(1, "submitted".into()).unwrap();
        let snapshot = d.commit_text(1, false).unwrap();
        d.edit(1, "newer typing".into()).unwrap();
        d.confirmed(1, snapshot);
        assert_eq!(d.source(1), Some("submitted"));
        assert_eq!(d.get(1).unwrap().text, "newer typing");
        assert!(!d.conflict(1));
        d.discard(1);
        d.flush().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn drafts_survive_reconnect_and_preserve_peer_conflicts() {
        let root = std::env::temp_dir().join(format!("rustudio-drafts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        {
            let mut d = DraftStore::new(root.clone());
            d.register(1, Some("{abc-123}"), "base".into()).unwrap();
            d.edit(1, "mine".into()).unwrap();
            d.receive(1, "theirs".into());
            assert!(d.conflict(1));
            assert!(d.commit_text(1, false).is_err());
            d.flush().unwrap();
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(root.join("abc-123.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        {
            let mut d = DraftStore::new(root.clone());
            d.register(99, Some("{abc-123}"), "theirs".into()).unwrap();
            assert_eq!(d.get(99).unwrap().text, "mine");
            assert!(d.conflict(99));
            assert_eq!(d.commit_text(99, true).unwrap(), "mine");
            d.submitted(99);
            d.flush().unwrap();
            d.receive(99, "mine".into());
            assert!(d.get(99).is_none());
            d.flush().unwrap();
        }
        assert!(!root.join("abc-123.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
