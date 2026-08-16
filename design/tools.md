# Writer tools

The writer exposes two disjoint tool sets over Bedrock Converse tool use.
Targeted editing gets record access plus proposal staging; whole-report
generation gets run-building tools and **no** record tools (its
snapshot is injected up front). Tool descriptions are contracts: ID-copying
rules, 0-based positions, character units, per-turn limits, and the
truncation-salvage behavior are all stated in the description or the
per-property schema the model sees. Schema ceilings are derived from the
`claria-core` domain validators so the wire can never drift from what the
domain accepts, and error results carry the serde/validation diagnostic
verbatim (structural messages are PHI-free) so the model's repair round is
cheap.

| Mode | Tools |
|---|---|
| Targeted edit | `list_record_files`, `read_record_file`, `propose_report_changes` |
| Whole report | `set_full_draft_title`, `write_full_draft_section`, `finish_full_draft` |

Every tool result is JSON with a success/error status. Persisted history is
sanitized: results carrying record or title text are reduced to digests and
`{"content_retained": false}` stubs after the turn, so the model re-reads
files rather than mining stale history.

---

## `list_record_files`

Input: `{}` (no parameters). Must be called before any read — only listed
filenames are readable.

```json
{}
```

Success result:

```json
{
  "files": [
    { "filename": "intake.txt", "readable": true, "source_too_large": false },
    { "filename": "basc3-parent.pdf", "readable": true, "source_too_large": false },
    { "filename": "raw-audio.wav", "readable": false, "source_too_large": true }
  ],
  "truncated": false
}
```

**Internal mapping.** Walks the client's record inventory through
`claria-records` (the same sidecar-visibility rules as the UI: a PDF/DOCX is
"readable" through its `.text` sidecar, printable UTF-8 originals directly).
`readable` reflects the 2 MiB source ceiling. The result is size-capped;
overflow drops trailing entries and sets `truncated`.

---

## `read_record_file`

| Field | Constraint |
|---|---|
| `filename` | required, 1–1024 chars, copied exactly from a list result |
| `offset` | optional, 0-based **Unicode characters** (not bytes), default 0 |
| `limit` | optional, 1–12,000 chars, default 8,000 |

All reads in a turn share a 48,000-character budget.

```json
{ "filename": "basc3-parent.pdf", "offset": 8000, "limit": 8000 }
```

Success result:

```json
{
  "filename": "basc3-parent.pdf",
  "text": "...8,000 characters of extracted Markdown...",
  "offset": 8000,
  "returned_characters": 8000,
  "total_characters": 21450,
  "next_offset": 16000,
  "sha256": "9f2c...e1"
}
```

`next_offset` is present while text remains — the model passes it back as
`offset` to continue. Error codes include `unknown_filename`,
`record_too_large` (2 MiB), `record_not_text`, `invalid_read_limit`,
`offset_out_of_range`, and `turn_read_limit_reached`.

**Internal mapping.** Resolves the sidecar-or-original key, fetches through
a per-turn read cache (one S3 GET per file per turn regardless of
pagination), draws down the 48k budget, records the file in the turn's
provenance list (the context pills in the UI), and digests the returned
slice. In persisted history the `text` field does not survive — only the
metadata and `{"content_retained": false}`.

---

## `propose_report_changes`

At most one successful call per turn — a second call fails. Nothing is
saved or applied until the user accepts.

| Field | Constraint |
|---|---|
| `summary` | required, 1–500 chars, shown to the user |
| `operations` | required, 1–25, applied in order against the accepted report |

Operations (discriminated on `kind`):

| `kind` | Fields |
|---|---|
| `set_title` | `title` (1–200) |
| `add_section` | `position` (0-based insertion index), `heading` (1–200), `blocks` (≤200) |
| `replace_section` | `section_id` (36-char UUID copied from the untrusted context), `heading`, `blocks` (≤200) — replaces the **whole** section, heading included; unchanged blocks must be restated |
| `remove_section` | `section_id` |

Blocks (discriminated on `kind`): `paragraph` (`text` ≤20,000 chars, plain
text, no markdown), `bullet_list` (`items` 1–100 × ≤2,000), `table` (`rows`
1–200 × ≤20 uniform cells of ≤5,000 chars, `has_header`, optional relative
`column_widths`).

```json
{
  "summary": "Rewrites the assessment results section with the BASC-3 parent and teacher findings.",
  "operations": [
    {
      "kind": "replace_section",
      "section_id": "3f9d2c1e-8a4b-4c6d-9e0f-112233445566",
      "heading": "Assessment Results",
      "blocks": [
        { "kind": "paragraph", "text": "The BASC-3 was completed by the parent and classroom teacher. Attention Problems T-scores were 72 and 68 respectively, in the clinically significant and at-risk ranges." },
        { "kind": "table", "rows": [["Scale", "Parent", "Teacher"], ["Attention Problems", "72", "68"]], "has_header": true }
      ]
    }
  ]
}
```

Success result:

```json
{
  "status": "pending_user_acceptance",
  "proposal_id": "b1c2d3e4-5f60-4788-99aa-bbccddeeff00",
  "base_revision": 4
}
```

**Internal mapping.** Operations are converted to domain operations and
dry-run against the accepted draft (`ReportDraft::preview`) — a failure
returns `invalid_proposal` with the validator's message verbatim. Success
stages a `ReportProposal` (id, base revision, model id, operations, and the
fully materialized proposed content) on the session. The UI renders the
review; **accept** applies the operations and lands a new revision,
**reject** discards. Either way the resolution joins the workspace history,
and the last 20 resolutions ride back to the model inside
`<untrusted_report_context>` on later turns.

---

## `set_full_draft_title`

```json
{ "title": "Psychoeducational Evaluation" }
```

Result: `{ "status": "title_staged" }`. Must be called before finalizing,
even when keeping the supplied title. Errors: `tool_not_available` (outside
full-draft mode), `full_draft_already_finalized`, `invalid_full_draft_title`.

**Internal mapping.** Sets the title on the isolated `FullDraftCandidate` —
never the workspace. Persisted history keeps only a `title_sha256` stub.

---

## `write_full_draft_section`

| Field | Constraint |
|---|---|
| `section_id` | a 36-char UUID copied exactly (template/report section), or `null` for a genuinely new section |
| `position` | 0-based final position, 0–100 |
| `heading` | 1–200 chars |
| `blocks` | 1–200, same block grammar as proposals |

```json
{
  "section_id": "3f9d2c1e-8a4b-4c6d-9e0f-112233445566",
  "position": 2,
  "heading": "Assessment Results",
  "blocks": [
    { "kind": "paragraph", "text": "Testing was completed over two sessions..." }
  ]
}
```

Success result:

```json
{
  "status": "section_staged",
  "section_id": "3f9d2c1e-8a4b-4c6d-9e0f-112233445566",
  "position": 2,
  "block_count": 1,
  "section_count": 7
}
```

Calling again with the returned `section_id` replaces the staged section.
An id that neither came from the host context nor from an earlier write is
rejected as `invented_section_id` — the anti-hallucination guard that keeps
template sections accounted for. `null` gets a server-assigned UUID.

**Internal mapping.** Moves the section to `drafted` in the run object and
writes the run back to S3 **before** this success result is returned — the
conversation never believes more than durable truth. The run's plan carries
one required entry per section present in the draft when generation started;
all of them must be drafted or skipped before finalization succeeds.

---

## `finish_full_draft`

```json
{ "summary": "Drafted all seven sections from the intake, BASC-3, and observation records." }
```

Success result:

```json
{
  "status": "full_draft_finalized",
  "section_count": 7,
  "base_revision": 0
}
```

Fails with the missing UUIDs listed when any required template/report
section was never written.

**Internal mapping.** Assembles the run's drafted sections in the order they
were written, re-inserts skipped ones as empty placeholders, and stages the
result; the host then saves it as **one atomic versioned revision**
(`replace_content`) with no proposal gate, stamps every run-authored section
with the run that wrote it, releases the session, and marks the run
completed. The loop's terminal guard also nudges the model once if it tries
to end the turn in prose before this tool succeeded.

---

## Cross-cutting rules

- **Error results are repair fuel.** `{"error": {"code", "message"}}` with
  the exact structural diagnostic; the model gets one corrective round per
  mismatch streak before the turn fails.
- **Truncation salvage.** A response cut at the output ceiling beside
  complete tool calls still executes those calls; the last result carries a
  rider telling the model to continue where it was cut.
- **Limits are model-visible.** Per-turn call ceilings, the read budget,
  and pagination all appear in descriptions — the model plans around them
  instead of discovering them by failing.
