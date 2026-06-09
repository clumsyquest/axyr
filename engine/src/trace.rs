//! Context-switch timeline ("what ran when").
//!
//! The firmware records each thread switch-in as `(timestamp, name_ptr)` in a
//! RAM ring buffer (a few instructions in the scheduler hot path — no printk,
//! safe). The host reads that ring over SWD (background read, no halt) and
//! reconstructs the timeline. The buffer layout matches the firmware:
//! `axyr_trace_head` (total writes) + `axyr_trace[N]` of two u32 each.

/// One recorded context switch.
pub struct TraceEntry {
    pub timestamp: u32,
    pub thread: String,
}

/// Number of ring slots (must match the firmware's `AXYR_TRACE_N`).
pub const RING_SLOTS: usize = 32;

/// Given the ring's `head` (total writes) and the raw words (`[ts, name_ptr]`
/// per slot), return the slot indices oldest-first for the entries to show.
pub fn ordered_indices(head: u32) -> Vec<usize> {
    let head = head as usize;
    let count = head.min(RING_SLOTS);
    (0..count).map(|k| (head - count + k) % RING_SLOTS).collect()
}

/// Render a timeline: each switch as cycles-since-first + thread name.
pub fn format_timeline(entries: &[TraceEntry]) -> String {
    if entries.is_empty() {
        return "No context-switch trace yet.".to_string();
    }
    let t0 = entries[0].timestamp;
    let mut out = String::from("Context-switch timeline (cycles since first, oldest first):\n");
    for e in entries {
        out.push_str(&format!("  +{:<11} {}\n", e.timestamp.wrapping_sub(t0), e.thread));
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_before_wrap() {
        // 3 writes, ring not yet full -> slots 0,1,2 oldest-first.
        assert_eq!(ordered_indices(3), vec![0, 1, 2]);
    }

    #[test]
    fn orders_after_wrap() {
        // head past the ring -> oldest is at head%N, then forward.
        let idx = ordered_indices(RING_SLOTS as u32 + 2);
        assert_eq!(idx.len(), RING_SLOTS);
        assert_eq!(idx[0], 2); // (34 - 32 + 0) % 32
        assert_eq!(*idx.last().unwrap(), 1); // newest
    }

    #[test]
    fn formats_deltas() {
        let entries = vec![
            TraceEntry { timestamp: 1000, thread: "main".into() },
            TraceEntry { timestamp: 1300, thread: "idle".into() },
        ];
        let out = format_timeline(&entries);
        assert!(out.contains("+0"));
        assert!(out.contains("+300"));
        assert!(out.contains("main"));
        assert!(out.contains("idle"));
    }
}
