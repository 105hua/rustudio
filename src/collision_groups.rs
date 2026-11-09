//! Collision groups (PhysicsService): the registered group names and which
//! pairs don't collide. In Roblox they are set on the server and replicated,
//! and both the physics and spatial queries (RaycastParams / OverlapParams
//! CollisionGroup) obey them; here one table is shared by the server and
//! client runtimes and the physics of a Play session.
//!
//! Roblox allows 32 groups (PhysicsService:GetMaxCollisionGroups), which maps
//! one to one onto rapier's 32-bit interaction groups: a part is a member of
//! its group's bit and collides with the bits of the groups that collide with
//! its own.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

pub const MAX_GROUPS: usize = 32; // "Cannot register more than 32 collision groups."
pub const DEFAULT: &str = "Default";
pub const MAX_NAME_LEN: usize = 100; // "Collision group name length cannot exceed %zu chars", 0x64

/// Studio's name check before registering, renaming or removing a group
/// empty, "Default" and over 100 bytes refused.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Collision group requires a valid name.".into());
    }
    if name == DEFAULT {
        return Err("Can not use/change the Default collision group.".into());
    }
    if name.len() > MAX_NAME_LEN {
        return Err(format!(
            "Collision group name length cannot exceed {MAX_NAME_LEN} chars"
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct CollisionGroups {
    /// Registered names; the index is the group's id (Default is 0).
    names: Vec<String>,
    /// Unordered pairs set not to collide (a ≤ b).
    off: HashSet<(String, String)>,
    /// Bumped on every change (the physics re-applies groups when it moves).
    pub version: u64,
}

pub type Shared = Arc<Mutex<CollisionGroups>>;

pub fn shared() -> Shared {
    Arc::new(Mutex::new(CollisionGroups::default()))
}

impl Default for CollisionGroups {
    fn default() -> Self {
        CollisionGroups {
            names: vec![DEFAULT.to_string()],
            off: HashSet::new(),
            version: 0,
        }
    }
}

fn pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

impl CollisionGroups {
    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn id(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }

    pub fn is_registered(&self, name: &str) -> bool {
        self.id(name).is_some()
    }

    /// PhysicsService:RegisterCollisionGroup: the name check,
    /// then at most 32 groups (Default counts), then no duplicate names.
    pub fn register(&mut self, name: &str) -> Result<(), String> {
        check_name(name)?;
        if self.names.len() >= MAX_GROUPS {
            return Err("Cannot register more than 32 collision groups.".into());
        }
        if self.is_registered(name) {
            return Err("Could not register collision group, the name already exists.".into());
        }
        self.names.push(name.to_string());
        self.version += 1;
        Ok(())
    }

    /// CollisionGroupSetCollidable: both groups registered
    /// (Default included).
    pub fn set_collidable_checked(&mut self, a: &str, b: &str, on: bool) -> Result<(), String> {
        if !self.is_registered(a) || !self.is_registered(b) {
            return Err("Both collision groups must be registered.".into());
        }
        self.set_collidable(a, b, on);
        Ok(())
    }

    pub fn unregister(&mut self, name: &str) {
        if name == DEFAULT {
            return;
        }
        if let Some(i) = self.id(name) {
            self.names.remove(i);
            self.off.retain(|(a, b)| a != name && b != name);
            self.version += 1;
        }
    }

    pub fn rename(&mut self, from: &str, to: &str) {
        if let Some(i) = self.id(from) {
            self.names[i] = to.to_string();
            self.off = self
                .off
                .drain()
                .map(|(a, b)| {
                    let r = |s: String| if s == from { to.to_string() } else { s };
                    pair(&r(a), &r(b))
                })
                .collect();
            self.version += 1;
        }
    }

    pub fn set_collidable(&mut self, a: &str, b: &str, on: bool) {
        let p = pair(a, b);
        let changed = if on {
            self.off.remove(&p)
        } else {
            self.off.insert(p)
        };
        if changed {
            self.version += 1;
        }
    }

    pub fn collidable(&self, a: &str, b: &str) -> bool {
        !self.off.contains(&pair(a, b))
    }

    /// Groups set not to collide with `group`.
    pub fn uncollidable_with(&self, group: &str) -> Vec<String> {
        self.off
            .iter()
            .filter_map(|(a, b)| {
                if a == group {
                    Some(b.clone())
                } else if b == group {
                    Some(a.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// (membership, filter) bits for a part in `group` (an unregistered
    /// name counts as Default, as Roblox treats it).
    pub fn bits(&self, group: &str) -> (u32, u32) {
        let name = if self.is_registered(group) { group } else { DEFAULT };
        let me = self.id(name).unwrap_or(0);
        let mut filter = 0u32;
        for (i, other) in self.names.iter().enumerate() {
            if self.collidable(name, other) {
                filter |= 1 << i;
            }
        }
        (1 << me, filter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_and_bits() {
        let mut g = CollisionGroups::default();
        assert!(g.register("WeaponRay").is_ok());
        assert!(g.register("PlayerClip").is_ok());
        assert!(g.register("PlayerClip").unwrap_err().contains("already exists"));
        assert!(g.register("Default").unwrap_err().contains("Default"));
        assert!(g.register("").is_err());
        assert!(g.register(&"x".repeat(101)).is_err());
        assert!(g.set_collidable_checked("WeaponRay", "Nope", false).is_err());
        g.set_collidable("WeaponRay", "PlayerClip", false);
        assert!(!g.collidable("PlayerClip", "WeaponRay"));
        assert!(g.collidable("WeaponRay", "Default"));
        let (m, f) = g.bits("WeaponRay");
        assert_eq!(m, 0b010);
        assert_eq!(f, 0b011);
        // Unknown names act as Default.
        assert_eq!(g.bits("Nope"), (1, 0b111));
        g.rename("PlayerClip", "Clip");
        assert!(!g.collidable("Clip", "WeaponRay"));
        g.unregister("Clip");
        assert_eq!(g.names().len(), 2);
        for i in 0..40 {
            let _ = g.register(&format!("G{i}"));
        }
        assert_eq!(g.names().len(), MAX_GROUPS);
        assert!(g.register("More").unwrap_err().contains("more than 32"));
    }
}
