# Memo Diarization & Paragraph Formatting

## Goal
When recording a session directly via the memo feature, produce formatted output with:
1. **Speaker diarization** — label different speakers (e.g., "Speaker 1:", "Speaker 2:")
2. **Paragraph formatting** — break the wall-of-text transcript into readable paragraphs

## Current State
- `claria-whisper` does greedy argmax decoding with `no_timestamps_token` — outputs a flat string with no timing or speaker info
- The frontend (`ClientRecord.tsx`) sends all accumulated PCM to `transcribe_memo` every 4 seconds, replaces `memoTranscript` with the result
- `TranscribeMemoResult` returns `{ text, language }` — no segments or timestamps
- The review modal lets users edit the transcript before saving as a `.text` file

## Approach: Bedrock Post-Processing

Whisper itself doesn't do diarization. There are two paths:
- **A) Local diarization model** — complex (needs pyannote or a custom Rust implementation), heavy, and not production-ready in candle
- **B) LLM post-processing** — after transcription is done, send the raw transcript to Bedrock with a prompt asking it to format with paragraphs and (if detectable from context) speaker labels

**Recommendation: Option B** — it's simple, uses existing Bedrock infra, and produces high-quality formatting. The user already has Bedrock access for chat. We add a "Format Memo" step between transcription and review.

## Implementation Plan

### 1. Add a memo formatting prompt to S3 (`claria-core`)
- Add `claria-prompts/memo-formatting.md` key to `s3_keys.rs`
- Default prompt instructs the LLM to: add paragraph breaks, identify/label speakers if multiple voices are apparent, preserve all original content

### 2. Add `format_memo` Tauri command (`claria-desktop`)
- New command: `format_memo(client_id, raw_transcript, model_id)` → `String`
- Loads the memo-formatting prompt from S3 (with fallback to a built-in default)
- Calls Bedrock (via `claria-bedrock`) with the prompt + raw transcript
- Returns the formatted text

### 3. Update `TranscribeMemoResult` to include segments (optional enhancement)
- Enable timestamps in Whisper decoding to get per-segment text with time offsets
- Include segment boundaries in the result so the formatter has timing info
- This helps the LLM identify speaker changes (pauses between speakers)

### 4. Update the frontend memo flow (`ClientRecord.tsx`)
- After final transcription (in `handleDoneMemo`), add a "Formatting..." step
- Call `format_memo` with the raw transcript
- Show both raw and formatted versions in the review modal
- User can toggle between raw/formatted, edit either, and save

### 5. Add prompt to the S3 key layout and provisioner
- Register the new prompt key in the manifest if needed for IAM

## File Changes

| File | Change |
|------|--------|
| `claria-core/src/s3_keys.rs` | Add `memo_formatting_prompt()` key |
| `claria-desktop/src/commands.rs` | Add `format_memo` command |
| `claria-desktop/src/main.rs` | Register `format_memo` in `collect_commands!` |
| `claria-whisper/src/lib.rs` | Enable timestamp tokens, return segments with time offsets |
| `claria-desktop-frontend/src/lib/tauri.ts` | Add `formatMemo` wrapper |
| `claria-desktop-frontend/src/pages/ClientRecord.tsx` | Add formatting step to memo flow, update review UI |

## Whisper Timestamp Enhancement (Step 3 detail)

Currently the decoder forces `no_timestamps_token`. To get segments with timestamps:
- Remove `no_timestamps_token` from the initial prompt tokens
- Parse `<|0.00|>` style timestamp tokens from the output
- Group text between timestamp pairs into `TranscribeSegment { start_secs, end_secs, text }`
- Return `TranscribeMemoResult { text, language, segments }` where `segments` is optional
- The frontend passes segments (with timing) to `format_memo` so the LLM can use pause duration as a diarization signal

## Default Memo Formatting Prompt

```
You are formatting a voice memo transcript. The transcript was recorded during a session and may contain one or more speakers.

Your task:
1. Break the transcript into clear paragraphs based on topic shifts and natural speech patterns.
2. If you can identify distinct speakers (from context clues, conversational patterns, or question/answer dynamics), label them as "Speaker 1:", "Speaker 2:", etc.
3. Preserve ALL original content — do not summarize, omit, or rephrase anything.
4. Fix obvious transcription errors only if you are very confident (e.g., homophones in context).
5. Add paragraph breaks where a speaker pauses, changes topic, or where a new speaker begins.

If timestamp information is provided (e.g., [0:00-0:30]), use gaps between segments as additional evidence for speaker changes.

Output only the formatted transcript — no commentary, headers, or metadata.
```
