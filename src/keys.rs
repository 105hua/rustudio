//! The key positions that are down, from egui's `Event::Key.physical_key`:
//! what Studio's movement keys and Roblox's KeyCodes mean, whichever layout
//! the keyboard uses (WASD moves from the keys labelled ZQSD on AZERTY).

use std::collections::BTreeSet;

/// The physical key positions held this frame.
#[derive(Default)]
pub struct PhysicalKeys {
    down: BTreeSet<egui::Key>,
}

impl PhysicalKeys {
    /// Folds this frame's raw input in (from the frame's input hook, before
    /// egui reads the events).
    pub fn ingest(&mut self, raw: &egui::RawInput) {
        for e in &raw.events {
            match e {
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed,
                    ..
                } => {
                    // No physical position (injected input): the logical key
                    // stands in, the way the self-test drives the editor.
                    let k = physical_key.unwrap_or(*key);
                    if *pressed {
                        self.down.insert(k);
                    } else {
                        self.down.remove(&k);
                    }
                }
                // A key released out of focus never sees its key-up.
                egui::Event::WindowFocused(false) => self.down.clear(),
                _ => {}
            }
        }
    }

    /// Whether the key position is down.
    pub fn down(&self, k: egui::Key) -> bool {
        self.down.contains(&k)
    }

    pub fn iter(&self) -> impl Iterator<Item = egui::Key> + '_ {
        self.down.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(k: egui::Key, physical: bool, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key: k,
            physical_key: physical.then_some(k),
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn tracks_presses_and_releases() {
        let mut keys = PhysicalKeys::default();
        keys.ingest(&egui::RawInput {
            events: vec![ev(egui::Key::W, true, true), ev(egui::Key::A, true, true)],
            ..Default::default()
        });
        assert!(keys.down(egui::Key::W) && keys.down(egui::Key::A) && !keys.down(egui::Key::S));
        keys.ingest(&egui::RawInput {
            events: vec![ev(egui::Key::A, true, false)],
            ..Default::default()
        });
        assert!(keys.down(egui::Key::W) && !keys.down(egui::Key::A));
        // A repeat keeps the key down.
        keys.ingest(&egui::RawInput {
            events: vec![ev(egui::Key::W, true, true)],
            ..Default::default()
        });
        assert!(keys.down(egui::Key::W));
    }

    #[test]
    fn injected_input_falls_back_to_the_logical_key() {
        let mut keys = PhysicalKeys::default();
        keys.ingest(&egui::RawInput {
            events: vec![ev(egui::Key::E, false, true)],
            ..Default::default()
        });
        assert!(keys.down(egui::Key::E));
    }

    #[test]
    fn losing_focus_releases_everything() {
        let mut keys = PhysicalKeys::default();
        keys.ingest(&egui::RawInput {
            events: vec![ev(egui::Key::Q, true, true)],
            ..Default::default()
        });
        keys.ingest(&egui::RawInput {
            events: vec![egui::Event::WindowFocused(false)],
            ..Default::default()
        });
        assert!(!keys.down(egui::Key::Q));
    }
}
