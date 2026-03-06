# Memo Diarization & Paragraph Formatting (On-Device)

## Goal
When recording a session directly via the memo feature, produce formatted output with:
1. **Speaker diarization** — label different speakers (e.g., "Speaker 1:", "Speaker 2:")
2. **Paragraph formatting** — break the wall-of-text transcript into readable paragraphs

Everything runs on-device. No Bedrock or cloud LLM dependency.

## Current State
- `claria-whisper` does greedy argmax decoding with `no_timestamps_token` — outputs a flat string with no timing or speaker info
- The frontend (`ClientRecord.tsx`) sends all accumulated PCM to `transcribe_memo` every 4 seconds, replaces `memoTranscript` with the result
- `TranscribeMemoResult` returns `{ text, language }` — no segments or timestamps
- The review modal lets users edit the transcript before saving as a `.text` file

## Approach: Whisper Timestamps + Heuristic Formatting

Whisper's tokenizer includes 1501 timestamp tokens (`<|0.00|>` through `<|30.00|>` at 0.02s resolution). By removing `no_timestamps_token` from the decoder prompt, Whisper naturally produces timestamped segments. We use the **gaps between segments** (silence/pauses) as signals for paragraph breaks and speaker changes.

### Why this works for sessions
In a therapy/clinical session, speakers naturally take turns with pauses between them. A short pause (< 2s) is mid-thought; a medium pause (2–4s) is a paragraph break; a long pause (> 4s) often signals a speaker change. These heuristics won't be perfect, but they produce useful structure that the user can refine in the review editor.

## Implementation Plan

### Step 1: Enable timestamps in Whisper (`claria-whisper`)

Modify `WhisperModel::transcribe()` to support a timestamp mode:

- Add a `transcribe_with_timestamps()` method (or a `timestamps: bool` parameter)
- When timestamps are enabled, **don't** push `no_timestamps_token` into the decoder prompt
- After decoding each 30s segment, parse the token sequence to extract timestamp-delimited chunks
- Each chunk becomes a `TranscribeSegment { start_secs: f64, end_secs: f64, text: String }`
- Offset segment timestamps by the 30s-chunk index (segment 0 starts at 0.0, segment 1 at 30.0, etc.)
- Return `TranscribeResult { text, language, segments: Vec<TranscribeSegment> }`

**Token parsing logic:**
- Timestamp tokens match `<|X.XX|>` in the tokenizer vocab
- Walk the decoded token list; when two consecutive timestamp tokens are found, everything between them is one segment
- Map timestamp token → f64 seconds (token text minus the `<|` / `|>` delimiters, parsed as f64)

### Step 2: Add formatting logic (`claria-whisper`)

Add a `pub fn format_segments(segments: &[TranscribeSegment]) -> String` function:

```
Rules:
1. If gap between segment[i].end and segment[i+1].start > SPEAKER_GAP (default 4.0s):
   - Insert blank line + "Speaker X:" label
   - Alternate speaker labels on each detected change
2. If gap > PARAGRAPH_GAP (default 2.0s) but ≤ SPEAKER_GAP:
   - Insert blank line (paragraph break, same speaker)
3. Otherwise:
   - Append text with a space separator (continuation)
```

Expose the gap thresholds as parameters so the frontend can let users tune them later.

### Step 3: Update `TranscribeMemoResult` (`claria-desktop`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct TranscribeSegment {
    pub start_secs: f64,
    pub end_secs: f64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct TranscribeMemoResult {
    pub text: String,
    pub language: Option<String>,
    pub segments: Vec<TranscribeSegment>,
    /// Formatted text with paragraph breaks and speaker labels
    pub formatted: String,
}
```

Update `transcribe_memo` command to:
- Call `transcribe()` with timestamps enabled
- Call `format_segments()` on the result
- Return both raw `text` and `formatted`

### Step 4: Update the frontend (`ClientRecord.tsx`)

- `memoTranscript` now holds the `formatted` text (with paragraphs/speaker labels)
- During live transcription (every 4s), show `formatted` in the live preview
- In the review modal, display the formatted text in the textarea
- User can edit freely before saving
- No new "formatting" loading step needed — it's instant (string processing, no model call)

### Step 5: Add diarization toggle to UI

- Add a small toggle/checkbox: "Detect speakers" (on by default)
- When off, `format_segments` still adds paragraph breaks but skips speaker labels
- Persist preference in memo recording state

## File Changes

| File | Change |
|------|--------|
| `claria-whisper/src/lib.rs` | Enable timestamp decoding, parse segments, add `format_segments()` |
| `claria-whisper/src/error.rs` | No change expected |
| `claria-desktop/src/commands.rs` | Update `TranscribeMemoResult` to include `segments` + `formatted`, call timestamp mode |
| `claria-desktop-frontend/src/pages/ClientRecord.tsx` | Use `formatted` text, add speaker detection toggle |
| `claria-desktop-frontend/src/lib/bindings.ts` | Auto-regenerated |

## Detailed Whisper Changes

### Token parsing for timestamps

In the Whisper tokenizer, timestamp tokens are IDs in a contiguous range. We detect them by:
1. On model load, scan the tokenizer vocab for tokens matching `<|0.00|>` through `<|30.00|>`
2. Store the token ID range (first_timestamp_id, last_timestamp_id)
3. During decoding, when a token falls in this range, compute `seconds = (token_id - first_timestamp_id) * 0.02`

### Decoder prompt change

Current: `[SOT, lang?, TRANSCRIBE, NO_TIMESTAMPS]`
Timestamp mode: `[SOT, lang?, TRANSCRIBE]` (omit `NO_TIMESTAMPS`)

The decoder will then naturally emit `<|start_time|> text tokens <|end_time|>` patterns.

### Segment extraction pseudocode

```rust
fn extract_segments(tokens: &[u32], chunk_offset_secs: f64) -> Vec<TranscribeSegment> {
    let mut segments = Vec::new();
    let mut current_start: Option<f64> = None;
    let mut current_tokens: Vec<u32> = Vec::new();

    for &token in tokens {
        if is_timestamp(token) {
            let time = timestamp_to_secs(token) + chunk_offset_secs;
            match current_start {
                None => {
                    current_start = Some(time);
                }
                Some(start) => {
                    let text = tokenizer.decode(&current_tokens, true);
                    if !text.trim().is_empty() {
                        segments.push(TranscribeSegment {
                            start_secs: start,
                            end_secs: time,
                            text: text.trim().to_string(),
                        });
                    }
                    current_start = None;
                    current_tokens.clear();
                }
            }
        } else if current_start.is_some() && !is_special(token) {
            current_tokens.push(token);
        }
    }
    segments
}
```

## Gap Thresholds

| Constant | Default | Meaning |
|----------|---------|---------|
| `PARAGRAPH_GAP_SECS` | 2.0 | Pause that triggers a paragraph break |
| `SPEAKER_GAP_SECS` | 4.0 | Pause that triggers a speaker change label |

These are reasonable defaults for clinical sessions. Users can adjust via the review editor if the automatic formatting isn't quite right.

## What This Does NOT Do

- **Voice fingerprinting** — we don't identify *who* is speaking, just *that* a different person likely started. True speaker ID would require a separate embedding model (e.g., ECAPA-TDNN) which is out of scope.
- **Perfect diarization** — pause-based heuristics work well for structured conversations (therapist ↔ client) but may mis-label in rapid back-and-forth. The editable review step handles this.
