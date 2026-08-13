#!/usr/bin/env python3
"""Regenerate the Word-template fixtures in this directory.

These packages are hand-authored to mirror what desktop Word actually emits
— rsid attributes, proofErr markers, custom-named heading styles carrying
w:outlineLvl, direct run formatting that differs from docDefaults, blank
spacer paragraphs, and content controls — none of which docx-rs produces.
The docx-rs-built templates in crates/claria-docx/tests/render.rs cannot
exercise those constructs, which is how the v0.22 underline/font/spacing
regressions shipped with green tests.

Run `python3 build_fixtures.py` from this directory and commit the .docx
outputs alongside this script.
"""

import zipfile

CONTENT_TYPES = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
</Types>
"""

ROOT_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>
"""

DOCUMENT_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>
"""

# Calibri docDefaults, while the visible body font is direct-formatted
# Garamond — the classic Word template shape where `Normal` was never edited.
STYLES = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:docDefaults>
<w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:cs="Calibri"/><w:sz w:val="22"/></w:rPr></w:rPrDefault>
<w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="259" w:lineRule="auto"/></w:pPr></w:pPrDefault>
</w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="Title">
<w:name w:val="Title"/><w:basedOn w:val="Normal"/>
<w:pPr><w:spacing w:after="300"/></w:pPr>
<w:rPr><w:rFonts w:ascii="Garamond" w:hAnsi="Garamond"/><w:b/><w:sz w:val="44"/></w:rPr>
</w:style>
<w:style w:type="paragraph" w:styleId="SectionHeading">
<w:name w:val="Section Heading"/><w:basedOn w:val="Normal"/>
<w:pPr><w:outlineLvl w:val="0"/><w:spacing w:before="240" w:after="120"/></w:pPr>
<w:rPr><w:rFonts w:ascii="Garamond" w:hAnsi="Garamond"/><w:b/><w:u w:val="single"/><w:sz w:val="28"/></w:rPr>
</w:style>
</w:styles>
"""

# The body layout mirrors a clinical evaluation template: custom-styled
# underlined section headings, Garamond direct formatting on body text, a
# label-run paragraph ("Assessment: "), blank spacer paragraphs, a results
# table with one deliberately empty cell, and an underlined signature line
# as the LAST body paragraph (the exemplar the v0.22 renderer cloned onto
# every generated paragraph).
CLINICAL_DOCUMENT = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body>
<w:p w:rsidR="00AB12CD" w:rsidRDefault="00AB12CD"><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r w:rsidRPr="00AB12CD"><w:t>Psychoeducational Evaluation</w:t></w:r></w:p>
<w:p w:rsidR="00AB12CE"><w:pPr><w:pStyle w:val="SectionHeading"/></w:pPr><w:proofErr w:type="spellStart"/><w:r><w:t>Reason for Referral</w:t></w:r><w:proofErr w:type="spellEnd"/></w:p>
<w:p w:rsidR="00AB12CF"><w:pPr><w:spacing w:after="240"/></w:pPr><w:r w:rsidRPr="00AB12CF"><w:rPr><w:rFonts w:ascii="Garamond" w:hAnsi="Garamond"/></w:rPr><w:t>Guardian requested evaluation for attention concerns.</w:t></w:r></w:p>
<w:p w:rsidR="00AB12D0"/>
<w:p w:rsidR="00AB12D1"><w:pPr><w:pStyle w:val="SectionHeading"/></w:pPr><w:r><w:t>Assessment Results</w:t></w:r></w:p>
<w:p w:rsidR="00AB12D2"><w:pPr><w:spacing w:after="240"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="Garamond" w:hAnsi="Garamond"/><w:b/><w:u w:val="single"/></w:rPr><w:t xml:space="preserve">Assessment: </w:t></w:r><w:r><w:rPr><w:rFonts w:ascii="Garamond" w:hAnsi="Garamond"/></w:rPr><w:t>Pending review.</w:t></w:r></w:p>
<w:tbl>
<w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/></w:tblPr>
<w:tblGrid><w:gridCol w:w="4675"/><w:gridCol w:w="4675"/></w:tblGrid>
<w:tr><w:tc><w:tcPr><w:shd w:val="clear" w:fill="D9D9D9"/></w:tcPr><w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Domain</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:shd w:val="clear" w:fill="D9D9D9"/></w:tcPr><w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Score</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:tc><w:p><w:r><w:rPr><w:rFonts w:ascii="Garamond" w:hAnsi="Garamond"/></w:rPr><w:t>Working Memory</w:t></w:r></w:p></w:tc><w:tc><w:p/></w:tc></w:tr>
</w:tbl>
<w:p w:rsidR="00AB12D3"/>
<w:p w:rsidR="00AB12D4"><w:r><w:rPr><w:u w:val="single"/></w:rPr><w:t>Clinician Signature: ______________________</w:t></w:r><w:r><w:rPr><w:u w:val="single"/></w:rPr><w:t xml:space="preserve">  Date: __________</w:t></w:r></w:p>
<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>
</w:body>
</w:document>
"""

# A fill-in form built from a content control: every paragraph sits inside
# <w:sdt><w:sdtContent>, so no flow span is a direct body child.
CONTENT_CONTROLS_DOCUMENT = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body>
<w:sdt>
<w:sdtPr><w:id w:val="123456"/></w:sdtPr>
<w:sdtContent>
<w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t>Psychoeducational Evaluation</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="SectionHeading"/></w:pPr><w:r><w:t>Reason for Referral</w:t></w:r></w:p>
<w:p><w:r><w:rPr><w:rFonts w:ascii="Garamond" w:hAnsi="Garamond"/></w:rPr><w:t>Guardian requested evaluation for attention concerns.</w:t></w:r></w:p>
</w:sdtContent>
</w:sdt>
<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>
</w:body>
</w:document>
"""


def build(name: str, document_xml: str) -> None:
    with zipfile.ZipFile(name, "w", zipfile.ZIP_DEFLATED) as package:
        package.writestr("[Content_Types].xml", CONTENT_TYPES)
        package.writestr("_rels/.rels", ROOT_RELS)
        package.writestr("word/_rels/document.xml.rels", DOCUMENT_RELS)
        package.writestr("word/styles.xml", STYLES)
        package.writestr("word/document.xml", document_xml)


if __name__ == "__main__":
    build("clinical-eval.docx", CLINICAL_DOCUMENT)
    build("content-controls.docx", CONTENT_CONTROLS_DOCUMENT)
    print("wrote clinical-eval.docx, content-controls.docx")
