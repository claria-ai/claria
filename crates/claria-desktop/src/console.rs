use std::{
    collections::VecDeque,
    fmt,
    sync::{Arc, Mutex},
    time::Instant,
};

use serde::{Deserialize, Serialize};
use tracing::{
    Event, Subscriber,
    field::Visit,
    span::{Attributes, Id, Record},
};
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};

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
        write!(
            f,
            "{} {} {}: {}",
            self.timestamp, self.level, self.target, self.message
        )
    }
}

/// One poll's worth of new console entries, addressed by a monotonic
/// sequence cursor so the 500ms UI poll ships only new lines.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ConsoleDelta {
    pub entries: Vec<ConsoleEntry>,
    /// Cursor to pass on the next poll.
    pub next_seq: u64,
    /// True when `entries` is the full buffer and must replace, not append —
    /// the requested cursor predates the buffer (lines rotated out unseen)
    /// or came from a previous app run.
    pub reset: bool,
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
    /// Sequence number of the entry at the front of `entries`. Every pushed
    /// entry gets the next number; rotation advances this.
    first_seq: u64,
}

impl BufferInner {
    /// Sequence number the next pushed entry will receive.
    fn next_seq(&self) -> u64 {
        self.first_seq + self.entries.len() as u64
    }
}

impl Default for ConsoleBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsoleBuffer {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(BufferInner {
                entries: VecDeque::new(),
                total_bytes: 0,
                first_seq: 0,
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
                buf.first_seq += 1;
            } else {
                break;
            }
        }
    }

    /// Entries at or after the sequence cursor `seq`.
    ///
    /// A cursor inside the buffered range appends (`reset: false`). A cursor
    /// older than the buffer start (entries rotated out between polls) or
    /// newer than the buffer end (cursor from a previous run) returns the
    /// full buffer with `reset: true` so the client replaces its copy.
    pub fn entries_since(&self, seq: u64) -> ConsoleDelta {
        let buf = self.inner.lock().expect("console buffer lock poisoned");
        let next_seq = buf.next_seq();
        if seq < buf.first_seq || seq > next_seq {
            return ConsoleDelta {
                entries: buf.entries.iter().cloned().collect(),
                next_seq,
                reset: true,
            };
        }
        let skip = usize::try_from(seq - buf.first_seq).unwrap_or(usize::MAX);
        ConsoleDelta {
            entries: buf.entries.iter().skip(skip).cloned().collect(),
            next_seq,
            reset: false,
        }
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

/// Per-span timing state stored in span extensions: creation time plus the
/// span's fields rendered as `key=value` pairs.
struct SpanTiming {
    started: Instant,
    fields: String,
}

/// Visitor that renders span fields as space-separated `key=value` pairs.
struct FieldVisitor {
    fields: String,
}

impl FieldVisitor {
    fn push(&mut self, name: &str, value: fmt::Arguments<'_>) {
        if !self.fields.is_empty() {
            self.fields.push(' ');
        }
        self.fields.push_str(name);
        self.fields.push('=');
        self.fields.push_str(&value.to_string());
    }
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        self.push(field.name(), format_args!("{value:?}"));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.push(field.name(), format_args!("{value}"));
    }
}

impl<S> Layer<S> for ConsoleLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
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

    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            let mut visitor = FieldVisitor {
                fields: String::new(),
            };
            attrs.record(&mut visitor);
            span.extensions_mut().insert(SpanTiming {
                started: Instant::now(),
                fields: visitor.fields,
            });
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        // Capture fields recorded after span creation (e.g. byte counts known
        // only once a response body has been read).
        if let Some(span) = ctx.span(id)
            && let Some(timing) = span.extensions_mut().get_mut::<SpanTiming>()
        {
            let mut visitor = FieldVisitor {
                fields: std::mem::take(&mut timing.fields),
            };
            values.record(&mut visitor);
            timing.fields = visitor.fields;
        }
    }

    /// Emit one entry per closed span carrying `elapsed_ms`, so
    /// `#[tracing::instrument]` timings reach the exported console log.
    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else { return };
        let extensions = span.extensions();
        let Some(timing) = extensions.get::<SpanTiming>() else {
            return;
        };

        let metadata = span.metadata();
        let elapsed_ms = timing.started.elapsed().as_millis() as u64;
        let mut message = metadata.name().to_string();
        if !timing.fields.is_empty() {
            message.push_str(&format!("{{{}}}", timing.fields));
        }
        message.push_str(&format!(" elapsed_ms={elapsed_ms}"));

        self.buffer.push(ConsoleEntry {
            timestamp: jiff::Timestamp::now().to_string(),
            level: metadata.level().to_string(),
            target: metadata.target().to_string(),
            message,
        });
    }
}
