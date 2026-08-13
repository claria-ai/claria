# Templates

A Claria writer template is an ordinary redacted Word document — a previous
report with client specifics removed, or a blank practice skeleton. There is
**no tag or merge-field system**: no `{{name}}` substitution engine, no
required placeholder grammar. The template is interpreted by the model
("smart" hydration) and its formatting is reused mechanically at export.
Those are two separate layers that never mix.

## Layer 1 — content: the template becomes data the model judges

`import_template` (claria-docx) converts the package into Claria's
structured report model: a title plus sections of paragraphs, bullet lists,
and tables. Import is defensive — macros, embedded objects, and encrypted
packages are rejected outright; images, headers/footers, text boxes, and
comments are omitted with warnings. Section boundaries come from a **style
catalog** resolved from the package's `styles.xml`: a paragraph is a heading
when its style is a literal `Heading*`/`Title`, carries an outline level, or
inherits one through the style's `basedOn` chain — so custom-named styles
like "Section Heading" section correctly. The importer and the export
renderer share this one classifier so they can never disagree about the
same package.

Applying a template to a session sets the working draft to that imported
content — headings **and** boilerplate body text. From then on the template
is *data*, delivered to the model inside `<untrusted_report_context>` with
`template_import` provenance (import revision, warnings, and whether
carryover has been reviewed for the current revision).

**Hydration is judgment, not substitution.** The whole-report prompt tells
the model to rewrite every supplied section — copying each `section_id`
exactly so no template section survives by omission — while preserving
useful headings, table structure, and row meaning, and leaving unknown
cells blank rather than inventing values. In practice the model carries
forward structural text (section headings, table header rows, standing
boilerplate that still applies) and replaces narrative content with
client-specific text drawn from the record snapshot. Underscores, `[Name]`,
`{{...}}`, and similar markers are treated the way a careful human reader
would treat them — as slots to fill — because the model reads them in
context, not because Claria parses them. (Claria does *count* common
placeholder markers — `{{`, `<<`, `[client`, `[name`, `[date`, `_____` —
but only as an import statistic surfaced in the UI, never for
substitution.)

## Layer 2 — formatting: the original package is the export

The applied template's exact source bytes are snapshotted immutably to
`report-authoring/{client}/templates/{sha256}.docx`. On export, Claria
copies that package wholesale — styles, fonts, numbering, headers, footers,
media, page setup all remain the original bytes — and rewrites only
`word/document.xml`:

- Unchanged content returns the source bytes exactly.
- Same-structure edits patch text in place, keeping every run property.
- Structural changes rebuild the body by cloning template paragraphs as
  formatting **exemplars**, chosen proportionally by kind (title, heading,
  body, list, table). Text unrelated to an exemplar's own words lands in
  its dominant run with direct bold/underline/italic stripped, so a field
  label or signature blank can't stamp its decoration onto generated prose;
  fonts, sizes, and paragraph spacing carry over.

Every export reports a fidelity level (`Exact`, `PatchedInPlace`,
`Reconstructed`, `PlainBodyFallback`) and the UI says so when formatting
could not be applied — a missing template snapshot or a content-control
(`w:sdt`) form falls back *visibly*, never silently. Content-control
templates are the main known limitation: their bodies cannot yet be walked
for exemplars.

## Trust rules

Templates and records are the untrusted half of every prompt, and the
boundary is enforced in three stacked ways:

1. **Instructions first, data after, in named delimiters.** The system
   prompt names the exact tags (`<untrusted_report_context>`,
   `<untrusted_record_context>`, chat's `<record_context>`) and states that
   everything inside them is data, never instructions. Content is escaped
   only where it could forge those delimiters — a document containing
   `</untrusted_report_context>` cannot close the region, while ordinary
   clinical text (`T-score >70`, `<3rd percentile`, `&`) passes through
   verbatim.
2. **The trust rules cannot be edited away.** The writer's system prompts
   are user-customizable in Preferences, but only the behavioral body: the
   *Untrusted data* and *Template carryover* sections are a fixed suffix
   appended at composition time, displayed read-only in the UI.
3. **The carryover rule targets the template-specific risk.** Because a
   template is often a previous client's redacted report, the fixed prompt
   orders the model to treat imported template facts as potentially
   belonging to a different person — never carrying a name, date, pronoun,
   diagnosis, or score forward unless the current user's instruction or
   current records support it — and the whole-report protocol's
   rewrite-every-section requirement backs that up structurally.

One boundary is procedural rather than technical: **redaction is the
clinician's responsibility at upload time.** Claria stores the template
bytes it is given; the UI instructs that only redacted templates be
uploaded, and the carryover rule plus mandatory section rewriting are the
defense-in-depth behind that instruction, not a substitute for it.

## Storage keys

| Key | Contents |
|---|---|
| `writer_templates/{uuid}.docx` | Account-wide template shelf: original bytes |
| `writer_templates/{uuid}.json` | Template name, size, upload date |
| `writer_templates/{uuid}.usage.json` | Best-effort use count |
| `report-authoring/{client}/templates/{sha256}.docx` | Immutable per-session snapshot used for export formatting |
