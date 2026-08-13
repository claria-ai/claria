# Word-template fixtures

Hand-authored OOXML packages that mirror what desktop Word emits — rsid
attributes, `proofErr` markers, a custom-named heading style
(`SectionHeading`) carrying `w:outlineLvl`, direct Garamond run formatting
over Calibri `docDefaults`, blank spacer paragraphs, a table with an empty
cell, an underlined signature line as the last body paragraph, and a
content-control (`w:sdt`) form. docx-rs cannot produce any of these
constructs, so templates built with it (as in
`crates/claria-docx/tests/render.rs`) cannot catch the formatting bugs
these fixtures pin.

Consumed by `crates/claria-docx/tests/template_fixtures.rs`.

| File | Exercises |
|---|---|
| `clinical-eval.docx` | exemplar cloning (underlined signature line last), custom heading classification, direct-format body fonts, blank-spacer positions, label-run paragraphs, empty table cell |
| `content-controls.docx` | `w:sdt`-wrapped body → template-fidelity fallback |

Regenerate with `python3 build_fixtures.py` in this directory and commit
the `.docx` outputs together with any script change.
