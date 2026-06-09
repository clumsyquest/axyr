//! Snapshot diff — "what changed since it was working".
//!
//! A recursive scalar diff over two system snapshots: changed values, additions,
//! and removals, by path. The always-changing `timeline` and `captured_at` are
//! skipped so the diff highlights what actually moved (a variable, a register, a
//! thread's CPU, the crash state).

use serde_json::Value;

const SKIP: &[&str] = &["timeline", "captured_at"];

/// Human-readable diff of `new` relative to `old`.
pub fn diff(old: &Value, new: &Value) -> String {
    let mut changes = Vec::new();
    walk("", old, new, &mut changes);
    if changes.is_empty() {
        "No changes since the last snapshot.".to_string()
    } else {
        format!("Changes since the last snapshot:\n{}", changes.join("\n"))
    }
}

fn walk(path: &str, old: &Value, new: &Value, out: &mut Vec<String>) {
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, bv) in b {
                if SKIP.contains(&k.as_str()) {
                    continue;
                }
                let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                match a.get(k) {
                    Some(av) => walk(&p, av, bv, out),
                    None => out.push(format!("  + {p} = {}", short(bv))),
                }
            }
            for k in a.keys() {
                if !b.contains_key(k) && !SKIP.contains(&k.as_str()) {
                    let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                    out.push(format!("  - {p}"));
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            for (i, bv) in b.iter().enumerate() {
                let p = format!("{path}[{i}]");
                match a.get(i) {
                    Some(av) => walk(&p, av, bv, out),
                    None => out.push(format!("  + {p} = {}", short(bv))),
                }
            }
            if a.len() > b.len() {
                out.push(format!("  - {path}[{}..]", b.len()));
            }
        }
        _ => {
            if old != new {
                out.push(format!("  {path}: {} -> {}", short(old), short(new)));
            }
        }
    }
}

fn short(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        _ => v.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reports_changed_added_removed() {
        let old = json!({ "state": {"core": "running"}, "variables": [{"name":"x","value":1}], "gone": 1 });
        let new = json!({ "state": {"core": "crashed"}, "variables": [{"name":"x","value":9}], "added": 2 });
        let d = diff(&old, &new);
        assert!(d.contains("state.core: running -> crashed"));
        assert!(d.contains("variables[0].value: 1 -> 9"));
        assert!(d.contains("+ added = 2"));
        assert!(d.contains("- gone"));
    }

    #[test]
    fn skips_timeline_and_reports_none() {
        let old = json!({ "timeline": [1, 2], "device": {"chip": "A"} });
        let new = json!({ "timeline": [3, 4, 5], "device": {"chip": "A"} });
        assert_eq!(diff(&old, &new), "No changes since the last snapshot.");
    }
}
