//! Studio's own API dump (app/api-dump.json, written by Studio
//! 0.744.0.7441077's `-API`: 942 classes, 3796 properties, 3374 functions,
//! 849 events, 652 enums), indexed for the gap telemetry and the
//! coverage table: which members exist, where they are declared, and what a
//! game script may touch (Security None).

use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Property,
    Function,
    Event,
    Callback,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Property => "property",
            Kind::Function => "function",
            Kind::Event => "event",
            Kind::Callback => "callback",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Member {
    pub kind: Kind,
    /// Security for reading / calling ("None" = any game script).
    pub security: String,
    /// Saved in place files (Serialization.CanLoad): the value a script reads
    /// is the stored one. Not saved = the engine works it out at run time.
    pub can_load: bool,
    pub read_only: bool,
    pub deprecated: bool,
    /// The Properties panel's category ("Appearance", "Data"...).
    pub category: String,
    /// Tagged Hidden or NotBrowsable: never listed in Properties.
    pub hidden: bool,
}

#[derive(Debug, Default)]
pub struct Class {
    pub superclass: Option<String>,
    pub members: HashMap<String, Member>,
    /// Declaration order, for stable listings.
    pub order: Vec<String>,
    pub creatable: bool,
    pub service: bool,
}

#[derive(Debug, Default)]
pub struct Api {
    pub classes: HashMap<String, Class>,
    pub class_order: Vec<String>,
    /// (enum, legacy item name) → the item's name now: the dump's
    /// LegacyNames (Enum.ContextActionPriority.Default is Medium).
    pub legacy_enum_items: HashMap<(String, String), String>,
    /// Every enum of the dump: name → (item, value). Enums Studio has that
    /// the reflection database lacks (ProjectionType, OrthographicMode…)
    /// come from here (ltypes::LEnumItem).
    pub enums: HashMap<String, Vec<(String, u32)>>,
}

fn dump_path() -> std::path::PathBuf {
    std::env::var_os("RUSTUDIO_API_DUMP")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/api-dump.json").into())
}

/// The dump, parsed once; None when the file is missing.
pub fn get() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(|| {
        let text = std::fs::read_to_string(dump_path()).ok()?;
        parse(&text)
    })
    .as_ref()
}

pub fn parse(text: &str) -> Option<Api> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut api = Api::default();
    for c in v.get("Classes")?.as_array()? {
        let name = c.get("Name")?.as_str()?.to_string();
        let tags: Vec<&str> = c
            .get("Tags")
            .and_then(|t| t.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
            .unwrap_or_default();
        let mut class = Class {
            superclass: c
                .get("Superclass")
                .and_then(|s| s.as_str())
                .filter(|s| *s != "<<<ROOT>>>")
                .map(str::to_string),
            creatable: !tags.contains(&"NotCreatable") && !tags.contains(&"Service"),
            service: tags.contains(&"Service"),
            ..Default::default()
        };
        for m in c.get("Members").and_then(|m| m.as_array()).into_iter().flatten() {
            let Some(mname) = m.get("Name").and_then(|n| n.as_str()) else {
                continue;
            };
            let kind = match m.get("MemberType").and_then(|t| t.as_str()) {
                Some("Property") => Kind::Property,
                Some("Function") => Kind::Function,
                Some("Event") => Kind::Event,
                Some("Callback") => Kind::Callback,
                _ => continue,
            };
            let security = match m.get("Security") {
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(o) => o
                    .get("Read")
                    .and_then(|r| r.as_str())
                    .unwrap_or("None")
                    .to_string(),
                None => "None".into(),
            };
            let mtags: Vec<&str> = m
                .get("Tags")
                .and_then(|t| t.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
                .unwrap_or_default();
            let can_load = m
                .get("Serialization")
                .and_then(|s| s.get("CanLoad"))
                .and_then(|b| b.as_bool())
                .unwrap_or(false);
            class.order.push(mname.to_string());
            class.members.insert(
                mname.to_string(),
                Member {
                    kind,
                    security,
                    can_load,
                    read_only: mtags.contains(&"ReadOnly"),
                    deprecated: mtags.contains(&"Deprecated"),
                    category: m
                        .get("Category")
                        .and_then(|c| c.as_str())
                        .unwrap_or("")
                        .to_string(),
                    hidden: mtags.contains(&"Hidden") || mtags.contains(&"NotBrowsable"),
                },
            );
        }
        api.class_order.push(name.clone());
        api.classes.insert(name, class);
    }
    for e in v.get("Enums").and_then(|e| e.as_array()).into_iter().flatten() {
        let Some(en) = e.get("Name").and_then(|n| n.as_str()) else {
            continue;
        };
        // An enum with no items still exists (AudioCaptureMode in 0.744).
        api.enums.entry(en.to_string()).or_default();
        for it in e.get("Items").and_then(|i| i.as_array()).into_iter().flatten() {
            let Some(name) = it.get("Name").and_then(|n| n.as_str()) else {
                continue;
            };
            if let Some(v) = it.get("Value").and_then(|v| v.as_u64()) {
                api.enums.entry(en.to_string()).or_default().push((name.to_string(), v as u32));
            }
            for old in it.get("LegacyNames").and_then(|l| l.as_array()).into_iter().flatten() {
                if let Some(old) = old.as_str() {
                    api.legacy_enum_items
                        .insert((en.to_string(), old.to_string()), name.to_string());
                }
            }
        }
    }
    Some(api)
}

impl Api {
    /// The member `name` of `class` or a superclass: (declaring class, member).
    pub fn lookup<'a>(&'a self, class: &str, name: &str) -> Option<(&'a str, &'a Member)> {
        let mut cur = Some(class);
        let mut guard = 0;
        while let Some(c) = cur {
            let (k, info) = self.classes.get_key_value(c)?;
            if let Some(m) = info.members.get(name) {
                return Some((k.as_str(), m));
            }
            cur = info.superclass.as_deref();
            guard += 1;
            if guard > 64 {
                break;
            }
        }
        None
    }

    /// Like `lookup`, with the dump's own strings: (declaring class, member
    /// name), borrowed from the parsed dump (no allocation on hot paths).
    pub fn lookup_names<'a>(&'a self, class: &str, name: &str) -> Option<(&'a str, &'a str)> {
        let mut cur = Some(class);
        let mut guard = 0;
        while let Some(c) = cur {
            let (k, info) = self.classes.get_key_value(c)?;
            if let Some((m, _)) = info.members.get_key_value(name) {
                return Some((k.as_str(), m.as_str()));
            }
            cur = info.superclass.as_deref();
            guard += 1;
            if guard > 64 {
                break;
            }
        }
        None
    }

    /// Whether `class` is `base` or derives from it.
    pub fn is_a(&self, class: &str, base: &str) -> bool {
        let mut cur = Some(class);
        let mut guard = 0;
        while let Some(c) = cur {
            if c == base {
                return true;
            }
            cur = self.classes.get(c).and_then(|i| i.superclass.as_deref());
            guard += 1;
            if guard > 64 {
                break;
            }
        }
        false
    }

    pub fn count(&self, kind: Kind, game_only: bool) -> usize {
        self.classes
            .values()
            .flat_map(|c| c.members.values())
            .filter(|m| m.kind == kind && (!game_only || m.security == "None"))
            .count()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn dump_counts_match_studio() {
        let Some(api) = super::get() else { return };
        use super::Kind::*;
        // Studio 0.744.0.7441077's own `-API` dump (RE-BRIDGE: 652 enums).
        assert_eq!(api.classes.len(), 942);
        assert_eq!(api.count(Property, false), 3796);
        assert_eq!(api.count(Function, false), 3374);
        assert_eq!(api.count(Event, false), 849);
        let (decl, m) = api.lookup("Part", "Anchored").unwrap();
        assert_eq!(decl, "BasePart");
        assert_eq!(m.kind, Property);
        assert!(api.lookup("Part", "Destroy").is_some());
        assert_eq!(api.enums.len(), 652);
        assert!(api.lookup("UserInputService", "IsWindowFocused").is_none());
    }
}
