use std::{
    collections::VecDeque,
    fmt,
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};
use tracing::{field::Visit, Event, Subscriber};
use tracing_subscriber::{layer::Context, Layer};

/// Maximum approximate byte size of the ring buffer (10 MB).
const MAX_BYTES: usize = 10 * 1024 * 1024;

/// A single log entry captured by the console ring buffer.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ConsoleEntry {
    pub timestamp: String,
    pub level: String,
    pub target: String,
    pub message: String,
}

impl ConsoleEntry {
    /// Approximate byte size of this entry for buffer cap accounting.
    fn byte_size(&self) -> usize {
        self.timestamp.len() + self.level.len() + self.target.len() + self.message.len()
    }
}

impl fmt::Display for ConsoleEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} {}: {}", self.timestamp, self.level, self.target, self.message)
    }
}

/// Thread-safe, size-capped ring buffer of log entries.
#[derive(Debug, Clone)]
pub struct ConsoleBuffer {
    inner: Arc<Mutex<BufferInner>>,
}

#[derive(Debug)]
struct BufferInner {
    entries: VecDeque<ConsoleEntry>,
    total_bytes: usize,
}

impl ConsoleBuffer {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(BufferInner {
                entries: VecDeque::new(),
                total_bytes: 0,
            })),
        }
    }

    fn push(&self, entry: ConsoleEntry) {
        let entry_size = entry.byte_size();
        let mut buf = self.inner.lock().expect("console buffer lock poisoned");
        buf.entries.push_back(entry);
        buf.total_bytes += entry_size;

        while buf.total_bytes > MAX_BYTES {
            if let Some(removed) = buf.entries.pop_front() {
                buf.total_bytes = buf.total_bytes.saturating_sub(removed.byte_size());
            } else {
                break;
            }
        }
    }

    /// Returns a clone of all buffered entries.
    pub fn entries(&self) -> Vec<ConsoleEntry> {
        let buf = self.inner.lock().expect("console buffer lock poisoned");
        buf.entries.iter().cloned().collect()
    }

    /// Formats all buffered entries as a plain-text string, one line per entry.
    pub fn to_text(&self) -> String {
        let buf = self.inner.lock().expect("console buffer lock poisoned");
        let mut out = String::new();
        for entry in &buf.entries {
            out.push_str(&entry.to_string());
            out.push('\n');
        }
        out
    }
}

/// A `tracing_subscriber::Layer` that captures events into a [`ConsoleBuffer`].
pub struct ConsoleLayer {
    buffer: ConsoleBuffer,
}

impl ConsoleLayer {
    pub fn new(buffer: ConsoleBuffer) -> Self {
        Self { buffer }
    }
}

/// Visitor that extracts the `message` field from a tracing event.
struct MessageVisitor {
    message: String,
}

impl MessageVisitor {
    fn new() -> Self {
        Self {
            message: String::new(),
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
        } else if self.message.is_empty() {
            // Fall back to first field if no explicit "message"
            self.message = format!("{}={:?}", field.name(), value);
        } else {
            self.message
                .push_str(&format!(" {}={:?}", field.name(), value));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else if self.message.is_empty() {
            self.message = format!("{}={}", field.name(), value);
        } else {
            self.message
                .push_str(&format!(" {}={}", field.name(), value));
        }
    }
}

impl<S: Subscriber> Layer<S> for ConsoleLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let mut visitor = MessageVisitor::new();
        event.record(&mut visitor);

        let entry = ConsoleEntry {
            timestamp: jiff::Timestamp::now().to_string(),
            level: metadata.level().to_string(),
            target: metadata.target().to_string(),
            message: visitor.message,
        };

        self.buffer.push(entry);
    }
}
