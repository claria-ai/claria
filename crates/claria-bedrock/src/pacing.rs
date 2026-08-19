//! How much streamed assistant text reaches the reader at once.
//!
//! `ConverseStream` hands back a few characters at a time. Forwarding every
//! one of those straight to the UI makes a reply twitch as it grows, re-lays
//! out Markdown on every frame, and leaves half-formed words and unclosed
//! emphasis on screen. Clinicians read that as jarring and unreadable.
//!
//! The pacer sits between the stream and the caller's delta callback and
//! decides when a chunk is worth showing. It is pure text bookkeeping, so it
//! can be driven directly in tests without a stream.

/// Cadence of incremental chat output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamPacing {
    /// Forward every delta exactly as the service produces it.
    Token,
    /// Hold text back until a paragraph closes, then release it whole.
    #[default]
    Paragraph,
    /// Forward nothing; the caller's return value carries the whole reply.
    Off,
}

/// Longest run of unbroken text [`StreamPacing::Paragraph`] will hold before
/// releasing it at the best break it can find.
///
/// A paragraph that never closes — a long single-paragraph answer, a fenced
/// code block, a bulleted list joined by single newlines — would otherwise
/// reach the reader only when the turn ends. Roughly a screenful of prose,
/// so the release still reads as "the next part arrived" rather than a tick.
const PARAGRAPH_FLUSH_CEILING: usize = 1_500;

/// Buffers stream deltas according to a [`StreamPacing`].
///
/// Every byte pushed in comes back out exactly once, in order: the paced
/// chunks concatenate to the original text under [`StreamPacing::Token`] and
/// [`StreamPacing::Paragraph`]. [`StreamPacing::Off`] emits nothing at all —
/// the caller's completed response is the only delivery.
#[derive(Debug)]
pub struct DeltaPacer {
    pacing: StreamPacing,
    held: String,
}

impl DeltaPacer {
    pub fn new(pacing: StreamPacing) -> Self {
        Self {
            pacing,
            held: String::new(),
        }
    }

    /// Absorb one delta, returning the text that is ready to show.
    pub fn push(&mut self, delta: &str) -> Option<String> {
        match self.pacing {
            StreamPacing::Token => (!delta.is_empty()).then(|| delta.to_string()),
            StreamPacing::Off => None,
            StreamPacing::Paragraph => {
                self.held.push_str(delta);
                let at = paragraph_break(&self.held).or_else(|| overlong_break(&self.held))?;
                let remainder = self.held.split_off(at);
                Some(std::mem::replace(&mut self.held, remainder))
            }
        }
    }

    /// Release whatever is still held. Called once the stream is complete, so
    /// the last partial paragraph is not lost.
    pub fn flush(&mut self) -> Option<String> {
        (!self.held.is_empty()).then(|| std::mem::take(&mut self.held))
    }
}

/// Byte offset just past the last paragraph break, if the held text has one.
fn paragraph_break(held: &str) -> Option<usize> {
    held.rfind("\n\n").map(|at| at + "\n\n".len())
}

/// Byte offset to break at when the held text has outgrown
/// [`PARAGRAPH_FLUSH_CEILING`] without closing a paragraph: the last line
/// break, else the last space, else all of it. Every candidate is ASCII, so
/// the offset is always a char boundary.
fn overlong_break(held: &str) -> Option<usize> {
    if held.len() < PARAGRAPH_FLUSH_CEILING {
        return None;
    }
    Some(
        held.rfind('\n')
            .or_else(|| held.rfind(' '))
            .map_or(held.len(), |at| at + 1),
    )
}
