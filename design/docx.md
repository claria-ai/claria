# DOCX modeling and rendering

Claria never lets the model touch OOXML. The document lives in memory as a
small, aggressively validated structure; edits arrive as typed operations
against it; and `.docx` bytes are produced **on demand at export only**,
derived entirely from that structure (plus, when a template is applied, the
template's original package). There is no stored docx that edits mutate —
the structure is the source of truth and every export is a fresh render.

## The in-memory model

```
ReportDraft
├─ revision: u64
├─ created_at / updated_at
└─ content: ReportContent
   ├─ title: String
   └─ sections: Vec<ReportSection>
      ├─ id: Uuid            ← stable identity edits address
      ├─ heading: String
      ├─ skipped: bool       ← deferred placeholder: empty body, elided from export
      └─ blocks: Vec<ReportBlock>
         ├─ Paragraph  { text }                        ← plain text, \n allowed
         ├─ BulletList { items: Vec<String> }          ← single level
         └─ Table      { rows: Vec<Vec<String>>,       ← uniform grid
                         has_header: bool,
                         column_widths: Option<Vec<u32>> }
```

Deliberate properties:

- **No inline formatting.** A paragraph is one string. There are no runs,
  no bold/italic spans, no hyperlinks, no styles in the model. Presentation
  is decided at render time (fixed house styles, or the template's).
- **Stable section identity.** Edits address sections by UUID, never by
  index or heading text, so concurrent hand-edits and model proposals can't
  target the wrong section.
- **Everything is bounded** by validators in `claria-core` (the same
  constants the tool schemas derive from): ≤100 sections, ≤200 blocks per
  section, ≤20,000 chars per paragraph, 1–100 bullet items of ≤2,000 chars,
  tables ≤200×20 with ≤5,000-char cells and consistent row widths, titles
  and headings ≤200 chars, ≤500,000 chars of content overall, and a
  printable-character discipline that rejects control characters Word XML
  cannot represent.

## How edits arrive and mutate the document

Two write paths, both funneling through the same validators:

**Targeted (proposal-gated).** The model's `propose_report_changes`
operations are dry-run against the current draft (`ReportDraft::preview`) —
this fully materializes the proposed content and rejects anything invalid
before the user ever sees it. The proposal (operations + materialized
result + base revision) is staged on the session. On **accept**, the
operations apply and the draft advances one revision; on **reject**,
nothing changes. A proposal staged against a stale base revision cannot
apply — the user's own edits win.

**Whole-report (atomic).** Full-draft generation builds an isolated
`FullDraftCandidate`; `write_full_draft_section` calls mutate only the
candidate, `skip_full_draft_section` records a user-directed deferral, and
`mark_section_failed` records a section the records could not support.
Finalization validates the complete content — including that every section
id present when generation started was written **or explicitly skipped** —
merges each skipped section back in as an empty `skipped` placeholder at
its template-relative position, and then saves it all as **one** revision
via `replace_content`. A `replace_section` that later writes content into a
deferred section clears its `skipped` flag.

The user's inline edits are a third writer: the frontend folds unsaved
edits into a draft-save, which is just another revision. Revisions are
persisted in the S3-versioned `workspace.json`; the history is listable and
any prior revision restores **as a new revision** (history is append-only,
never rewritten).

## Rendering out on demand

Export loads a snapshot (draft + optional template source) and picks one of
two renderers in `claria-docx`. Both renderers first drop `skipped`
sections — a deferred section holds its place in the draft and the canvas,
but the exported document contains no trace of it until content is written
into it:

**Plain renderer** (`render_report`) — no template. Builds a fresh package
with docx-rs: Times New Roman 12pt `Normal`, centered bold `Title`, bold
`Heading1` with keep-next, bullets through one real numbering definition
(id 42), tables with a shaded bold header row, `cantSplit` rows, relative
column widths scaled into the printable width, Letter pages with 1-inch
margins. `\n` inside a paragraph becomes separate Word paragraphs; inside a
bullet item it becomes a line break.

**Template renderer** (`render_report_with_template`) — the applied
template's stored package is copied wholesale (styles, fonts, numbering,
headers, footers, media, page setup stay original bytes) and only
`word/document.xml` is rewritten:

1. **Exact** — visible content matches the template: source bytes returned.
2. **Patched in place** — same structure, changed text: only text events
   rewritten; every run and paragraph property survives.
3. **Reconstructed** — structure changed: each target block clones a
   template paragraph/table of the same kind as its formatting *exemplar*
   (chosen proportionally along the document), text lands in the exemplar's
   dominant run, and direct bold/underline/italic is stripped when the new
   text is unrelated to the exemplar's own words. Headings classify through
   a style catalog (literal `Heading*`, outline levels, `basedOn` chains),
   shared with the importer. Blank spacer paragraphs interleave at the
   proportional boundaries. Bullets without a template exemplar bring their
   numbering definition into the package.
4. **Plain-body fallback** — a body the walker can't handle (content
   controls): generated body formatting inside the template package.

Every export reports which level it achieved (`TemplateRenderFidelity`) and
the UI says so when formatting was reduced — degradation is visible, never
silent.

**Preview ≠ export.** The in-app canvas renders block text through a
Markdown component; the exporters write strings verbatim. The prompt
forbids markdown in paragraph text, but nothing strips it — `**bold**`
would look bold in the preview and export as literal asterisks. This
divergence is a known sharp edge and one argument for the inline-markup
option below.

## How a template becomes sections

Carving is style-driven, and only falls back to appearance when the styles
say nothing at all.

**Tier 1 — applied heading styles.** A paragraph opens a section when its
style resolves to a heading through `StyleCatalog`: the normalized styleId
starts with `heading`, the style's display name starts with `heading`, or
the style definition carries `<w:outlineLvl>` — each followed up the
`basedOn` chain. A template that applied Word's heading styles gets exactly
the carve it asks for, and nothing below can change that.

**Tier 2 — appearance, only when tier 1 promoted nothing.** Most real
clinical templates never apply a heading style: their headers are body text
someone bolded, and the whole document used to import as one section
holding everything. When no paragraph carried a heading style, a second
pass promotes paragraphs that are **emphasized** (every text run bold, or
no lowercase letters), **label-shaped** (≤80 characters and not ending in
`. ? ! : ; ,`), and **lettered** (at least one letter, so a typed rule of
underscores is not a heading). All three are load-bearing: length or the
missing full stop alone each match ordinary sentences, and a field label
ending in a colon is not a section.

Two guards decide whether that result is adopted at all — at least
`MIN_INFERRED_HEADINGS` (2), and at most `MAX_INFERRED_HEADING_DENSITY`
(60%) of paragraphs. A template that strictly alternates heading and
paragraph is already half headings and is a good structure, so the bar sits
above one half; past it there is more heading than content, which is what a
document set entirely in bold looks like. Failing either guard keeps the
single invented section.

An inferred carve always emits `SectionsInferredFromFormatting`, because it
is a guess about someone's document rather than a reading of it, and the
plan gate lets the clinician correct the section list before drafting
spends anything.

**Paragraph-level `<w:outlineLvl>` is deliberately not consulted.** It
looks like the better signal — an authored claim rather than an appearance
— but real templates set it indiscriminately. One field example carried it
on 140 paragraphs including table cells and blank lines, marked each
heading *and* the body paragraph after it, and still missed half the real
headings.

`claria-docx-cli` (`cargo run -p claria-docx-cli -- <file.docx>`) reports
what both tiers did to a package, including which style rule fired and
which paragraphs read as headings but carry no heading style.

## Feature inventory

**In the model — the LLM can author these:**

| Feature | Notes |
|---|---|
| Title, section headings | One heading level |
| Paragraphs | Plain text; `\n` = real line/paragraph breaks |
| Bullet lists | Single level, unordered |
| Tables | Uniform grid, header-row flag, relative column widths, blank cells |
| Section ordering | Via `position` / operation order |

**Not in the model, but preserved at export via templates** (the model
never sees or controls these — the template package supplies them):

fonts and run styles · paragraph spacing and indentation · heading
typography (incl. underlined headers) · blank spacer paragraphs · page
size/margins · headers and footers · embedded media · table borders and
shading · numbering glyphs and indents.

**Unsupported — would break the LLM abstraction** (each would force the
model from semantic content into layout/typography decisions, and most
would break the exemplar-patching contract, which is paragraph-granular):

inline character formatting (bold/italic/underline spans) · hyperlinks ·
nested and ordered lists · multiple heading levels (import flattens >1 with
a warning) · merged cells (`gridSpan`/`vMerge` — import rejects such
tables) · nested tables (omitted with warning) · per-cell formatting ·
text boxes · footnotes/endnotes · fields (page numbers, TOC, cross-refs) ·
section breaks and multi-column layout · content controls (`w:sdt`) ·
tracked changes and comments.

**Unsupported — no interest or security-rejected:**

images inserted by the model · equations · SmartArt/charts · embedded OLE
objects (rejected at import) · macros/VBA (rejected) · encrypted packages
(rejected) · forms.

## Growing the model's formatting powers

Could the model bold a word? Yes — but "bold" is the tip of a design
choice, because inline formatting is the one feature class that breaks the
current paragraph-granular contract everywhere at once: the block schema,
the domain validators, the preview renderer, and — hardest — the template
exemplar patcher, which today distributes *plain characters* into an
exemplar's existing runs. An inline bold span means synthesizing new runs
with toggled properties inside a cloned template paragraph.

Three viable shapes for inline emphasis:

- **A. Markdown subset in paragraph text.** Permit `**bold**`/`*italic*`,
  parsed at render. Cheapest: zero schema change, the model is natively
  fluent, and it *converges* preview and export (the preview already
  renders markdown). Costs: escaping ambiguity when clinical text contains
  literal asterisks, a parser plus validation in `claria-docx`, and
  run-splitting in the template path.
- **B. Typed rich runs.** `Paragraph { text }` becomes
  `Paragraph { runs: [{ text, bold?, italic?, underline? }] }`. Explicit
  and validated with no parsing ambiguity, and import could round-trip
  template emphasis. Costs: every paragraph becomes a nested array (more
  tokens per proposal, more schema surface, more model error modes) and a
  workspace schema migration.
- **C. Semantic emphasis classes.** Inline or block-level tags like
  `term`, `score`, `finding` that render through *styles* — the plain
  renderer maps them to bold/italic, a template maps them to its own
  character styles. Most Word-idiomatic (styles over direct formatting),
  keeps the model in semantic territory, and template typography stays in
  charge. Cost: a vocabulary to design and explain.

C is the best fit for Claria's philosophy (the model decides *what*,
presentation layers decide *how*); A is acceptable sugar if preview/export
convergence is the goal; B is the fallback when exact control matters more
than token cost.

Cheaper wins exist below the inline threshold — these are **block-level**,
so they fit the existing exemplar model (a new block kind maps to "clone a
template exemplar of this kind, or generate one"):

| Feature | Sketch |
|---|---|
| Ordered (numbered) lists | New list flavor; numbering-merge machinery already exists |
| Heading levels 2–3 | `level` on sections/headings; style catalog already understands `Heading2+`; import currently flattens them |
| Page break before a section | Boolean → `w:pageBreakBefore`; template-friendly |
| Paragraph alignment | `center`/`right` for signature lines and captions |
| Table column alignment | Right-align score columns — high clinical value, tiny schema cost |
| Table captions | A styled paragraph bound above/below a table |

Whatever gets added must ride the standing invariants: the ceiling lives in
the `claria-core` validator with the wire schema derived from it; the
template renderer needs an exemplar-or-generated answer for the new kind
(and a fixture asserting it); import should round-trip it or warn; the
preview renderer must show it; and the golden prompt-shape tests must pin
the new schema so it can't silently regress. The trust boundary is
untouched — formatting is content-layer, and none of this gives the
document a way to smuggle instructions.
