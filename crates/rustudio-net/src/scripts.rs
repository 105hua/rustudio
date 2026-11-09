//! Native LiveScripting atomic source updates (Studio 0.746).
use crate::replicator::PropValue;
use md5::{Digest, Md5};

pub fn client_configuration() -> PropValue {
    PropValue::String("{\"version\":\"1\"}".into())
}

/// Atom positions are zero: replacing the whole draft avoids splitting Unicode.
/// The MD5 precondition makes the server reject a concurrent source change.
pub fn replace(request: i64, guid: &str, base: &str, text: &str) -> Vec<PropValue> {
    let mut atoms = Vec::new();
    if !base.is_empty() {
        atoms.push(serde_json::json!({"type":1,"position":0,"contents":base}));
    }
    if !text.is_empty() {
        atoms.push(serde_json::json!({"type":0,"position":0,"contents":text}));
    }
    let op = serde_json::json!({"payload":{"atoms":atoms},"timestamps":{"local":0,"remote":0}})
        .to_string();
    vec![
        PropValue::Int(request),
        PropValue::String(guid.into()),
        PropValue::String(op),
        PropValue::String(format!("{:x}", Md5::digest(base.as_bytes()))),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_keeps_unicode_and_a_compare_and_set_precondition() {
        let args = replace(7, "guid", "return '🦊'\n", "return 'é'\n");
        let PropValue::String(op) = &args[2] else {
            panic!()
        };
        let op: serde_json::Value = serde_json::from_str(op).unwrap();
        assert_eq!(op["payload"]["atoms"][0]["contents"], "return '🦊'\n");
        assert_eq!(op["payload"]["atoms"][0]["type"], 1);
        assert_eq!(op["payload"]["atoms"][1]["contents"], "return 'é'\n");
        let empty = replace(8, "guid", "", "x");
        assert_eq!(
            empty[3],
            PropValue::String("d41d8cd98f00b204e9800998ecf8427e".into())
        );
        assert_ne!(args[3], replace(7, "guid", "changed", "return 'é'\n")[3]);
    }
}
