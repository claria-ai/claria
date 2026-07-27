# Transcript body fixtures

The `.text` sidecar body has two parsers: `parse_transcript_body` /
`format_transcript_body` in `crates/claria-transcribe/src/lib.rs`, and
`parseBody` / `renderBody` in
`claria-desktop-frontend/src/lib/transcript.ts`. Rust writes the body when a
transcription lands; TypeScript reads and rewrites it when the user edits a
segment. Nothing in the type system connects them, so they can drift silently
and the symptom is a mangled transcript.

These files are the contract. Every case is read by both
`crates/claria-transcribe/tests/body_format.rs` and
`claria-desktop-frontend/src/lib/transcript.test.ts` — the same bytes, not two
copies — so a change to one parser that the other doesn't get fails on both
sides at once.

They live at the repository root rather than under either tree because neither
side owns the format, and reaching into the other's test assets would imply
one does.

## Layout

Each case is a pair:

- `<name>.txt` — the body, exactly as it would sit in S3
- `<name>.expected.json` — what both implementations must produce from it

The JSON is deliberately language-neutral; each side maps its own
representation onto it.

```json
{
  "description": "why this case exists",
  "speaker_labels": ["Clinician", "Patient"],
  "segments": [
    {
      "id": "seg_0001",
      "speaker_label": "Clinician",
      "language_code": null,
      "start_seconds": 0,
      "end_seconds": 4,
      "text": "How are you feeling today?",
      "translation": null
    }
  ],
  "rendered": "…"
}
```

- `speaker_labels` — distinct speaker labels in first-seen order.
- `segments` — the parse result. `speaker_label` is the resolved label, not
  the internal speaker id, since Rust interns labels to `spk_N` and TypeScript
  keys them by the label string.
- `rendered` — the body after a parse-then-render round trip. Not always equal
  to the input: a hand-typed ASCII hyphen normalises to an en dash, and a
  segment with no speaker renders as `Speaker`.

## Adding a case

Write the pair, then run `cargo test -p claria-transcribe` and `npm test` in
`claria-desktop-frontend`. Both suites discover files by globbing this
directory, so there is nothing to register.
