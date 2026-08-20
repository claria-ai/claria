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
like "Section Heading" section correctly. When that pass finds no headings
at all — the clinical template that marks its sections in bold and applies
no styles — an appearance rule promotes short, unpunctuated, bold-or-caps
paragraphs instead, and is refused unless it finds at least two of them and
they stay under 60% of the document (a page set entirely in bold is not a
table of contents). The importer and the export renderer share this one
carve so they can never disagree about the same package: the export
classifies the template against the sections the import already found
rather than re-deriving its own.

Applying a template to a session sets the working draft to that imported
content — headings **and** boilerplate body text. From then on the template
is *data*, delivered to the model inside `<untrusted_report_context>` in
targeted editing and `<untrusted_template_context>` in whole-report
drafting, both carrying `template_import` provenance (import revision,
warnings, and whether carryover has been reviewed for the current revision).

**Hydration is judgment, not substitution.** The whole-report prompt tells
the model to decide every planned section — copying each `section_id`
exactly and either rewriting it, explicitly skipping it when the plan or the
user's guidance defers it to a later pass, or marking it failed when the
records cannot support it, so no template section survives by omission —
while preserving useful headings, table structure, and row meaning, and
leaving unknown cells blank rather than inventing values. A
skipped section keeps its heading as an empty deferred placeholder; its
boilerplate body is dropped rather than carried, and export omits the
section until content is written into it. In practice the model carries
forward structural text (section headings, table header rows, standing
boilerplate that still applies) and replaces narrative content with
client-specific text drawn from the record snapshot. Underscores, `[Name]`,
`{{...}}`, and similar markers are treated the way a careful human reader
would treat them — as slots to fill — because the model reads them in
context, not because Claria parses them. (Claria does *count* common
placeholder markers — `{{`, `<<`, `[client`, `[name`, `[date`, `_____` —
but only as an import statistic surfaced in the UI, never for
substitution.)

**Bracketed authoring notes reach the model, as form.** Clinical templates
carry instructions their author wrote for whoever fills them in — "[a one
sentence statement of why the client was referred]", "[Delete the
subsections of the tests that were not administered]". Import extracts those
`[…]` segments verbatim per section into `ReportSection::template_directives`
(at most 8 per section, 500 characters each), and the host hands them to the
planner and the drafting branch as its own named field. They steer *form*
only — how long a section runs, how it must open, which subsection to drop —
and the planner is told to scope a row that fits them, so a one-sentence
directive stops producing a five-sentence plan. They are never evidence and
never a fact about the client. The serial whole-report path does not supply
them; the per-section parallel path does.

## Layer 2 — formatting: the original package is the export

The applied template's exact source bytes are snapshotted immutably to
`report-authoring/{client}/templates/{sha256}.docx`. On export, Claria
copies that package wholesale — styles, fonts, numbering, headers, footers,
media, page setup all remain the original bytes — and rewrites only
`word/document.xml`:

- Unchanged content returns the source bytes exactly.
- Same-structure edits patch text in place, keeping every run property.
- Structural changes rebuild the body **section by section**. The export
  imports the template and classifies its paragraphs against that carve, so
  it can never disagree with the import about what a heading is. Draft
  sections align to template sections by heading text, renamed headings
  included; a section's blocks map in order by kind onto the paragraphs the
  author wrote for it, so a rewritten heading keeps the author's own bold
  paragraph. Blocks that outgrow a section clone a formatting **exemplar**
  from within it, then from the nearest same-kind paragraph document-wide.
  Text unrelated to a cloned exemplar's own words lands in its dominant run
  with direct bold/underline/italic stripped — except on headings, where
  that decoration is what makes the paragraph a heading — so a field label
  or signature blank can't stamp its decoration onto generated prose; fonts,
  sizes, and paragraph spacing carry over.
- Content the accepted draft does not hold is never re-emitted. Template
  material between recognised paragraphs survives only when it carries no
  content (blank spacers, `sectPr`, bookmarks); a table the walker cannot
  represent is dropped rather than scattered through the export.
- Merged cells (`gridSpan`/`vMerge`) are read as the rectangle they
  describe, by one rule both the import and the export call, so the score
  tables clinical templates are built from reach the model — which can then
  fill them or delete them, and could do neither while they were dropped. A
  filled table is written back into the template's own merged cells; the
  merges survive.

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
   `<untrusted_template_context>`, `<untrusted_record_context>`,
   `<plan_context>`, chat's `<record_context>`) and states that
   everything inside them is data, never instructions. Content is escaped
   only where it could forge those delimiters — a document containing
   `</untrusted_report_context>` cannot close the region, while ordinary
   clinical text (`T-score >70`, `<3rd percentile`, `&`) passes through
   verbatim. The one carve-out is `template_directives`: because the host
   extracts them deterministically and presents them as its own field
   rather than as template prose, the prompts grant them authority over
   form alone — never over tools, records, or the rules themselves. A
   template already dictates the document's entire shape, so letting its
   bracketed notes say how long a section runs is not new authority;
   carrying facts across clients stays forbidden.
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
