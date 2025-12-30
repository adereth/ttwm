//! Event tracing for debugging ttwm.
//!
//! Provides a ring buffer of recent events for debugging and replay.
//! Agents can query the event log via IPC to understand what happened.

use std::collections::VecDeque;
use std::time::Instant;


use crate::ipc::EventLogEntry;
use crate::state::StateTransition;

/// Maximum number of events to keep in the trace buffer
const DEFAULT_MAX_ENTRIES: usize = 1000;

/// Event tracer with ring buffer storage
#[allow(dead_code)]
pub struct EventTracer {
    entries: VecDeque<EventLogEntry>,
    max_entries: usize,
    sequence: u64,
    start_time: Instant,
}

#[allow(dead_code)]
impl EventTracer {
    /// Create a new event tracer with default capacity
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_MAX_ENTRIES)
    }

    /// Create a new event tracer with specified capacity
    pub fn with_capacity(max_entries: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(max_entries),
            max_entries,
            sequence: 0,
            start_time: Instant::now(),
        }
    }

    /// Get the current timestamp in milliseconds since tracer start
    fn timestamp(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }

    /// Trace an X11 event
    pub fn trace_x11_event(&mut self, event_type: &str, window: Option<u32>, details: &str) {
        self.add_entry(event_type.to_string(), window, details.to_string());
    }

    /// Trace a state transition
    pub fn trace_transition(&mut self, transition: &StateTransition) {
        let (event_type, window, details) = match transition {
            StateTransition::WindowManaged { window, frame } => {
                ("window_managed".to_string(), Some(*window), format!("frame={}", frame))
            }
            StateTransition::WindowUnmanaged { window, reason } => {
                let reason_str = serde_json::to_string(reason).unwrap_or_else(|_| "unknown".to_string());
                ("window_unmanaged".to_string(), Some(*window), reason_str)
            }
            StateTransition::FocusChanged { from, to } => {
                ("focus_changed".to_string(), *to, format!("from={:?}", from))
            }
            StateTransition::TabSwitched { frame, from, to } => {
                ("tab_switched".to_string(), None, format!("frame={} from={} to={}", frame, from, to))
            }
            StateTransition::FrameSplit { original_frame, new_frame, direction } => {
                ("frame_split".to_string(), None, format!("original={} new={} dir={}", original_frame, new_frame, direction))
            }
            StateTransition::SplitResized { split, old_ratio, new_ratio } => {
                ("split_resized".to_string(), None, format!("split={} {:.2}->{:.2}", split, old_ratio, new_ratio))
            }
            StateTransition::WindowMoved { window, from_frame, to_frame } => {
                ("window_moved".to_string(), Some(*window), format!("from={} to={}", from_frame, to_frame))
            }
            StateTransition::FrameRemoved { frame } => {
                ("frame_removed".to_string(), None, format!("frame={}", frame))
            }
        };
        self.add_entry(event_type, window, details);
    }

    /// Trace an IPC command
    pub fn trace_ipc(&mut self, command: &str, result: &str) {
        self.add_entry("ipc_command".to_string(), None, format!("cmd={} result={}", command, result));
    }

    /// Add an entry to the trace buffer
    fn add_entry(&mut self, event_type: String, window: Option<u32>, details: String) {
        // Remove oldest entry if at capacity
        if self.entries.len() >= self.max_entries {
            self.entries.pop_front();
        }

        self.sequence += 1;
        self.entries.push_back(EventLogEntry {
            sequence: self.sequence,
            timestamp_ms: self.timestamp(),
            event_type,
            window,
            details,
        });
    }

    /// Get the last N entries
    pub fn get_last(&self, n: usize) -> Vec<EventLogEntry> {
        let start = if self.entries.len() > n {
            self.entries.len() - n
        } else {
            0
        };
        self.entries.iter().skip(start).cloned().collect()
    }

    /// Get all entries
    pub fn get_all(&self) -> Vec<EventLogEntry> {
        self.entries.iter().cloned().collect()
    }

    /// Clear the trace buffer
    pub fn clear(&mut self) {
        self.entries.clear();
        self.sequence = 0;
    }

    /// Get the number of entries in the buffer
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for EventTracer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_x11_event() {
        let mut tracer = EventTracer::new();
        tracer.trace_x11_event("MapRequest", Some(12345), "new window");

        let entries = tracer.get_all();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event_type, "MapRequest");
        assert_eq!(entries[0].window, Some(12345));
    }

    #[test]
    fn test_ring_buffer_overflow() {
        let mut tracer = EventTracer::with_capacity(3);

        tracer.trace_x11_event("event1", None, "");
        tracer.trace_x11_event("event2", None, "");
        tracer.trace_x11_event("event3", None, "");
        tracer.trace_x11_event("event4", None, "");

        let entries = tracer.get_all();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].event_type, "event2");
        assert_eq!(entries[2].event_type, "event4");
    }

    #[test]
    fn test_get_last() {
        let mut tracer = EventTracer::new();

        for i in 0..10 {
            tracer.trace_x11_event(&format!("event{}", i), None, "");
        }

        let last_3 = tracer.get_last(3);
        assert_eq!(last_3.len(), 3);
        assert_eq!(last_3[0].event_type, "event7");
        assert_eq!(last_3[2].event_type, "event9");
    }

    #[test]
    fn test_sequence_numbers() {
        let mut tracer = EventTracer::new();

        tracer.trace_x11_event("a", None, "");
        tracer.trace_x11_event("b", None, "");
        tracer.trace_x11_event("c", None, "");

        let entries = tracer.get_all();
        assert_eq!(entries[0].sequence, 1);
        assert_eq!(entries[1].sequence, 2);
        assert_eq!(entries[2].sequence, 3);
    }
}
