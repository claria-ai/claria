# Writer tools

The writer exposes three disjoint tool sets over Bedrock Converse tool use.
Targeted editing gets record access plus proposal staging; whole-report
generation gets run-building tools and **no** record tools (its
snapshot is injected up front); analysis gets the structured-submission
tools the planner answers through. Tool descriptions are contracts: ID-copying
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
| Whole report | `set_full_draft_title`, `write_full_draft_section`, `skip_full_draft_section`, `mark_section_failed`, `finish_full_draft` |
| Analysis | `submit_section_plan`, `submit_resume_plan`, `submit_review_rows` |

The analysis set is different in kind: it is sent with `tool_choice` forcing
exactly one of its tools, so the answer is structure by construction rather
than structure parsed out of prose. The whole set is declared on every
analysis request even though only one tool can be called, because a differing
tool list would move the tools tier of the prompt cache and cost each role
the record corpus the others paid to cache.

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
| `citations` | optional, ≤20 × `{filename, quote}`; filename copied from the record snapshot, quote a verbatim 10–300-character span |

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
  "citation_count": 0,
  "section_count": 7
}
```

Calling again with the returned `section_id` replaces the staged section.
An id that neither came from the host context nor from an earlier write is
rejected as `invented_section_id` — the anti-hallucination guard that keeps
template sections accounted for. `null` gets a server-assigned UUID. A
citation naming a file outside the run's record snapshot is
`unknown_citation_file`; a quote outside 10–300 characters is
`invalid_citation_quote`. Quotes are **not** matched against record text
here — that is the completion gate's job.

**Internal mapping.** Moves the section to `drafted` in the run object and
writes the run back to S3 **before** this success result is returned — the
conversation never believes more than durable truth. The run's plan carries
one required entry per section present in the draft when generation started;
all of them must be drafted, skipped, or failed before finalization
succeeds. Three consecutive rejected writes for the same section return
`section_attempts_exhausted` and mark that section failed with the last
structural diagnostic, so one unwritable section cannot consume the run's
call budget.

---

## `skip_full_draft_section`

```json
{ "section_id": "3f9d2c1e-8a4b-4c6d-9e0f-112233445566" }
```

Result: `{ "status": "section_skipped", "section_id": "..." }`. Valid only
for a supplied section; an invented id is `invented_section_id`, and a
section already written is `section_already_written` (a skip never overrides
a write).

**Internal mapping.** Moves the section to `skipped` in the run object and
writes the run back before returning. When the plan was edited by a human
and marks the section `draft`, the first skip returns a `plan_conflict`
error carrying the plan row verbatim; a second skip of the same section is
accepted and records `skip diverged from the approved plan` on it.

---

## `mark_section_failed`

| Field | Constraint |
|---|---|
| `section_id` | a 36-char UUID copied from `plan_context` |
| `reason` | 1–500 chars, PHI-free — what is missing, not what the records say |

Result: `{ "status": "section_marked_failed", "section_id": "..." }`.

**Internal mapping.** Moves the section to `failed` with the reason on it
and writes the run back before returning. Assembly then leaves that
section's base-revision content unchanged, so the run completes with the
gap visible rather than stalling on it.

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
  "skipped_section_count": 1,
  "failed_section_count": 0,
  "base_revision": 0
}
```

Fails with the missing UUIDs listed when any planned section was never
written, skipped, or marked failed.

**Internal mapping.** Assembles the run's drafted sections in the order they
were written, re-inserts skipped ones as empty placeholders and failed ones
as their base-revision content, and stages the result; the host then saves
it as **one atomic versioned revision**
(`replace_content`) with no proposal gate, stamps every run-authored section
with the run that wrote it, releases the session, and marks the run
completed. The loop's terminal guard also nudges the model once if it tries
to end the turn in prose before this tool succeeded.

---

## `submit_section_plan`

Forced. Exactly one row per section in `<untrusted_template_context>`, in
template order.

| Field | Constraint |
|---|---|
| `section_id` | a 36-char UUID copied from the supplied structure |
| `action` | `draft` or `skip` |
| `scope` | 1–600 chars — what the section must assert, or why it is skipped |
| `evidence` | ≤8 × `{filename, quote (10–300 chars), relevance?}` |

**Internal mapping.** Coverage and identity are hard: a missing row, a
duplicate, or an invented ID returns the offending IDs as an error
`tool_result` and forces the tool again, once. Evidence is soft: each quote
is resolved against the pinned record corpus by whitespace-normalized
substring search, and one that does not resolve is dropped with a
`unresolved_quote:{filename}` warning on the plan rather than failing it. A
`draft` row left with no resolved evidence adds
`no_resolved_evidence:{section_id}`. The plan lands on the run
`awaiting_approval`, unapproved, for the clinician to edit.

---

## `submit_resume_plan`

Forced. Runs only when a resume carries new instructions; a resume without
them is decided in code (drafted → keep, skipped → skip, everything else →
draft) and never calls a model.

| Field | Constraint |
|---|---|
| `section_id` | a 36-char UUID copied from the supplied state table |
| `decision` | `keep`, `rewrite`, `draft`, or `skip` |
| `reason` | 1–300 chars, read by the host and not stored |
| `scope` | required for `rewrite` and `draft` |
| `evidence` | required for `rewrite` |

**Internal mapping.** `keep` is rejected for any section whose durable state
is not `drafted`. A `rewrite` or `draft` decision clears that section back to
pending — including one that already landed — so the writer really replaces
it instead of the finisher treating it as already decided.

---

## `submit_review_rows`

Forced, once per review branch. One union tool for all seven properties, with
an identical `tool_choice` on every branch: a per-property tool would move the
tools tier of the prompt cache and cost each branch the corpus prefix its
sibling paid to write. Which fields are legal for which property is stated in
the description and enforced by the host, not by the shape.

| Field | Constraint |
|---|---|
| `property` | one of the seven; must equal the property the instruction named |
| `rows` | exactly one row per drafted section, in the order the drafted-section block lists them |
| `rows[].status` | `no_issues` or `findings` — an omitted section is a validation failure, not silence |
| `rows[].findings[].span` | `{quote, block_index?}` — the index is a hint the host searches first, not an address it trusts |
| `rows[].findings[].replacement` | required on the four style properties, rejected on the three consistency ones |

**Internal mapping.** Every quote is resolved to a `(block_index, char range)`
against the section's own text by whitespace-normalized search; one that does
not resolve discards the finding rather than anchoring it somewhere plausible,
and one that resolves twice anchors the first occurrence and appends a
`duplicate_anchor` note. The anchor revision and the model ID are stamped from
the request the host sent, never read from the answer. A branch that fails
validation twice fails alone: the other six keep their findings, and the
property it covered contributes no coverage row.

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
