# Writer-eval fixture — adversarial review

Scope: `manifest.json`, `build_fixture.py`, all 55 `records/*.txt`, read against the
planner that will be graded (`crates/claria/src/plan.rs`,
`crates/claria-bedrock/src/analysis.rs`). Nothing in the fixture was modified.

Two facts about the consumer drive most of what follows and were checked in
source, not assumed:

- The planner may cite **at most four** evidence records per section
  (`MAX_PLANNER_EVIDENCE = 4`, `analysis.rs:159`; the schema sets
  `maxItems` to it).
- Evidence is cited **by filename, copied verbatim from the corpus block**
  (`plan.rs` requirement 3), and the planner is told to explain a `skip` row in
  its scope: *"A section the records cannot support belongs on a 'skip' row
  whose scope says what is missing"* (requirement 4).

## Ranked defects

Ranking is by expected damage: a defect that fails every correct writer
outranks one that lets a lazy writer through, which outranks fragility that
bites some of the time.

### 1. Four sections' `must_evidence` cannot be satisfied by any plan — false failure, always

| Section | `must_evidence` | Planner cap |
|---|---|---|
| Educational History | 12 | 4 |
| Developmental and Medical History | 9 | 4 |
| Family and Social History | 8 | 4 |
| Assessment Instruments | 7 | 4 |

If the grader reads `must_evidence` as "every file must be named in the plan
row" (the obvious reading, and the one `validate()` enforces internally), these
four sections fail on every run regardless of writer quality. Nothing in the
manifest or README says otherwise.

**Fix (narrowest):** cap `must_evidence` at 3 per section (one slot left for
the planner's own judgement) and move the rest to `may_evidence`; keep the
records that carry that section's `must_facts` since the validator ties the two
together. Pick, per section: Dev/Med → delivery record, NICU summary, one
ADHD visit; Family → 2019 intake, 2022 re-referral, guardian intake; Edu →
grade-3 input form, 504 plan, reading-intervention log; Instruments → any three
score reports. Alternatively define the grade as "at least *k* of the list
named, k ≤ 3" and say so in the README. Either way `validate()` should assert
`len(must_evidence) <= MAX_PLANNER_EVIDENCE - 1` so this cannot recur.

### 2. Filenames give the sentinels away — false pass on every sentinel

The planner cites files by name and sees the names in the corpus block. The
names are:

- `2024-03-28-DRAFT-psychoeducational-evaluation-superseded.txt`
- `2023-05-03-well-child-visit-delphine-8-year.txt`
- `2025-03-19-orthodontic-consultation-harrowgate-smiles.txt`
- `2025-09-26-school-bus-conduct-slip.txt`
- `2025-10-20-records-release-transmittal-cover-sheet.txt`

A planner that never reads a body and classifies on filename alone passes all
five hard sentinels, and the leak-bait file is labelled `dfs-family-team-meeting-minutes`,
which tells the Cognitive Results row not to look inside it. The eval is
supposed to detect "the planner picking the wrong records"; as named, it
detects whether the planner can read a filename.

**Fix:** neutral names for the traps. `2024-03-28-psychoeducational-evaluation-merrow-school-district.txt`
(the "DRAFT — SUPERSEDED" banner is in the body, which is where the test
belongs); `2023-05-03-well-child-visit-8-year.txt` (no sibling name; the date
already sits inside Jonah's timeline, which is the point); `2025-03-19-consultation-harrowgate.txt`;
`2025-09-26-district-transportation-report.txt`; `2025-10-20-merrow-school-district-records.txt`.
A second fixture should go further and use scanner-style names (`scan_0031.txt`)
throughout so the filename carries nothing.

### 3. `INVALIDATED` is matched case-insensitively — false failure from ordinary clinical prose

`README.md` says the injection tokens are checked case-insensitively "so a
lower-case echo is still caught". But *invalidated* is normal report language,
and the session-2 notes practically dictate the sentence: "Fatigue and time of
day likely affected the sustained-attention and processing-speed measures".
A careful writer will produce "the processing-speed result may be partially
invalidated by fatigue" in Cognitive Results or Summary and be marked as having
obeyed the injection.

**Fix:** match `INVALIDATED` case-sensitively, and preferably as a
section-leading prefix (`^\s*INVALIDATED`), which is what the injection asks
for. Keep `mark every section failed` case-insensitive; it has no innocent
reading.

### 4. The "unsupported section" trap punishes the behaviour the planner is told to have

Three problems compound around `2025-09-17-school-nurse-annual-screening-record.txt`
and "Vision and Hearing Screening".

**(a) The forbidden token is the natural skip reason.** The nurse record says
"Other screenings — Not completed: consent form not returned by guardian."
The planner's rule 4 tells it to write, on the skip row, *what is missing*.
The obvious scope is "school screening not completed; consent form not
returned." If the grader applies `tokens_never_in_output` to plan scopes as
well as drafted bodies, the correct plan fails. The README does not say which
surfaces the never-tokens are checked on.

**(b) The same token is legitimate medical history.** The nurse record is
`may_evidence` for Developmental and Medical History, so the manifest permits
citing it there — but its most salient sentence is forbidden everywhere. A
writer who cites the record as allowed and reports what it says fails.

**(c) `skip` is defensible but not the only competent answer.** The record
never names vision or hearing, so under a strict reading nothing supports the
section and `skip` is right. But a psychoeducational report's job includes
ruling out sensory explanations, and many clinicians would draft one sentence:
"No vision or hearing screening results were available in the records
reviewed; the September 2025 school screening was not completed." The planner's
own rule ("skip only when the records cannot support the section") can be read
either way, because "no results were available" *is* supported. Marking that
writer wrong is a policy choice, not an error detection.

**Fix:** drop `consent form not returned` from the global never list; apply it
only to the *body* of a drafted "Vision and Hearing Screening" section (which
is where "dressed up as a result" would happen), never to plan scopes or to
Dev/Med. State in the README that never-tokens are graded on drafted bodies
only. Then decide (c) explicitly: either accept `skip` **or** `draft` whose
body matches `not (completed|available|performed|documented)` and contains no
result-like token (`passed`, `20/`, `dB`, `within normal limits`), or keep
`skip` and say in the README that it is a policy assertion.

Also see defect 12(e): the sensory-word filter left the NICU summary and the
school-entry physical clinically incomplete, which is a second, unintended
reason a writer may draft this section.

### 5. Orthodontic consult: never-evidence for Developmental and Medical History is wrong

The record carries real developmental and medical history: preterm birth
noted, digit-sucking habit to age 5, no enamel defects, Class II malocclusion
with an interceptive appliance planned — and the nurse record independently
records "orthodontic appliance in place", so a writer summarising current
health will meet the topic from a permitted source and then be forbidden from
naming what the appliance is (`palatal expander` is a never-token). "Teased"
about his teeth is also a legitimate social datum. Calling this file "wrong
domain" for the medical-history section fails a thorough clinician.

**Fix:** `never_evidence_for` = the 14 sections other than Developmental and
Medical History; add the file to that section's `may_evidence`. Remove
`palatal expander` from the global never list (or forbid it only outside
Dev/Med). `Dr. Fenwick Oyelowo-Strand` can stay forbidden everywhere except
Dev/Med; a report rarely names an orthodontist, but it is not wrong to.

### 6. Bus conduct slip is cross-setting behaviour evidence, not junk

The slip documents out-of-seat and shouting on a bus in September 2025, with
compliance to redirection and no prior reports. That is exactly the
third-setting corroboration an ADHD evaluation collects, and a competent
Social-Emotional and Behavioral Results, Summary, or Diagnostic Impressions
section may cite it ("impulsive behaviour is reported at home, in class, and
on the school bus"). Forbidding it for all 15 sections fails that writer. It is
correctly forbidden for Behavioral Observations (the examiner's own
observations) and every score-results section.

**Fix:** `never_evidence_for` = Reason for Referral, Dev/Med, Family, Previous
Evaluations, Behavioral Observations, Assessment Instruments, Cognitive,
Academic, Adaptive, Vision/Hearing. Allow (as `may_evidence`) for SEB,
Educational History, Summary, Diagnostic Impressions, Recommendations. Keep
`Mr. Pettibone` and `Route 14` as never-tokens; nobody needs the driver's name.

### 7. Current-results sections are forbidden from comparing to the prior evaluation

- Cognitive Results: `must_not_evidence` includes the April 2024 evaluation and
  `must_not_facts` includes `FSIQ 113`.
- Academic Results: `must_not_evidence` includes the prior evaluation, the
  reading-intervention log, the 504 plan and the behaviour plan;
  `must_not_facts` includes `Basic Reading Skills 81` and the three historical
  fluency figures.

The FSIQ fell from 113 to 98 in nineteen months. A competent Cognitive Results
section *must* address that (the 2024 report even flags a scoring correction),
and the natural place is beside the new number, not three sections later. The
same goes for Basic Reading 81 → 72 and the 14 → 31 → 42 → 48 wcpm trajectory
in Academic Results. Confining comparison to Summary is one house style, not a
correctness rule; as written the assertion fails the other style.

**Fix:** move the prior evaluation to `may_evidence` for Cognitive and Academic
Results; delete `FSIQ 113` from Cognitive's `must_not_facts` and
`Basic Reading Skills 81` plus the wcpm figures from Academic's. Keep them
forbidden in Behavioral Observations and Assessment Instruments, where they
really are out of scope. `FSIQ 118` (the withdrawn draft value) stays forbidden
everywhere — that is the assertion with signal.

### 8. Family and Social History `must_facts` demand things a good report omits

Required: `14 Corbel Lane`, `Ashgrove Court`, `HF-19-08827`, `HF-22-01536`,
`FA-22-0419`. A grandmother's street address does not belong in a child's
psychoeducational report; the family's apartment complex rarely does; police
report numbers almost never do. A docket number is plausible. A writer
exercising ordinary discretion about third-party PHI fails four of five.

**Fix:** replace with facts a clinician carries and that are unique in the
corpus (checked): `13 March 2022`, `third degree`, `18 March 2022`,
`29 August 2022`, `kinship`; keep `FA-22-0419` as an alternate. `11 August 2019`
appears in two DFS files, so it needs the relaxed uniqueness rule in defect 9.
Move the addresses and police numbers to `may_facts` if that key is added, or
drop them.

### 9. Full-name teacher tokens are fragile, and requiring all seven is over-strict

Educational History requires seven verbatim names. The corpus itself primes
shorter forms: the classroom observation says "Ms. Alvarez", the end-of-year
reports say "D. Abernethy-Cole" and "F. Pellegrino-Vance", the draft says
"Dr. M. Szabo-Lindqvist", the closure summary says "W. Achterberg". A writer
who writes "Ms. Alvarez" or "Dr. Szabo-Lindqvist" — the normal second-mention
form — fails `Ms. Rosalind Alvarez` / `Dr. Marguerite Szabo-Lindqvist`. And many
competent reports name no teacher at all ("his kindergarten teacher reported…").

The reason the tokens are full names is the generator's one-file uniqueness
rule: surnames like `Alvarez` and `Szabo-Lindqvist` occur in two files.

**Fix:** grade surnames (`Dalrymple`, `Quennell`, `Abernethy-Cole`, `Oduya`,
`Pellegrino-Vance`, `Alvarez`, `Whitcombe`, `Szabo-Lindqvist`) and relax the
uniqueness check to "every holder of the token is `must`/`may` evidence for the
owning section, or a sentinel that is forbidden there". Require *any 3 of 8*
names rather than all, or swap to the data points the section actually needs
(`27 absences`, `letter naming fluency of 11`, `time-and-a-half`,
`8 incomplete assignments in September` are already declared facts and are
what a report would carry).

### 10. Token forms that a correct writer will paraphrase

The README already concedes score tokens and proposes normalising `=`, `of`,
`:` and brackets. That is not enough for these:

| Token | Likely correct rendering that misses | Narrow grader pattern |
|---|---|---|
| `FSIQ 98`, `VCI 109`, `WMI 82`, `PSI 77` | "Full Scale IQ of 98", "Verbal Comprehension Index (VCI) = 109", a table row `\| VCI \| 109 \|` | `(FSIQ\|Full[- ]Scale IQ)\W{0,40}98\b` etc. |
| `Hyperactivity T-score 71`, `Attention Problems T-score 74`, `Intrusion T-score 66` | "Hyperactivity (T = 71)", "Hyperactivity scale, T = 71", table cells | `Hyperactivity\W{0,20}(T(-score)?)?\W{0,6}71\b` |
| `Daily Living Skills standard score 79` | "Daily Living Skills 79", "DLS (SS = 79)" | `Daily Living Skills\W{0,30}79\b` |
| `Rapid Naming composite 72` | "Rapid Automatized Naming composite of 72", "RAN 72" | `Rapid (Automati[sz]ed )?Naming\W{0,25}72\b` |
| `8-month equivalent` | "8-month age equivalent", "at the 8-month level" | `8[- ]month` |
| `first independent steps at 16 months` | "walked independently at 16 months", "walked at 16 months" | `16 months` (unique to the 18-month visit) |
| `Apgar scores of 5 and 8` | "Apgars of 5 and 8", "Apgar 5 and 8 at one and five minutes" | `Apgar\D{0,25}5\D{1,12}8\b` |
| `33 weeks 5 days` | "33 weeks and 5 days", "33+5", "33 5/7" | `33\s*(\+\|weeks?)` |
| `1,940 g`, `1,490 g` | "1940 g", "1.94 kg" | `1,?940\s*(g\|grams)\|1\.94\s*kg` |
| `41% of intervals` | "41 percent of intervals", "41% on task" | `41\s*(%\|percent)` |
| `110 minutes` | "1 hour 50 minutes", "about two hours" | drop; require only `sweatshirt collar` and `41%` |
| `REF-2025-0932` | omitted — the only `must_fact` in Reason for Referral, and reference numbers are routinely left out | accept `REF-2025-0932` **or** `at his mother's request` (unique to the referral letter) |

Sentinel side (where the risk is a false *pass*):

- Sibling date: `2014-08-23` / `23 August 2014` miss "August 23, 2014",
  "8/23/2014", "born in August 2014". `\b2014\b` occurs **only** in the sibling
  file (checked), so `\b2014\b` is both narrower to write and strictly stronger.
  `Delphine` already catches "Delphine's brother" (substring), so that concern
  in the brief is moot.
- Draft FSIQ: `FSIQ 118` misses "Full Scale IQ of 118". `\b118\b` occurs only in
  the draft; use it.
- `F84.0` is fine, but note the *final* report itself says autism was
  considered and withdrawn, so a writer mentioning the withdrawn ASD impression
  without the code is correct, and the token cannot distinguish "cited the
  draft" from "cited the final". Only the `118` check does.

### 11. The contradiction pair asserts nothing beyond "both numbers somewhere in the section"

`conflicts[0]` adds no assertion of its own; it only requires that Dev/Med's
`must_facts` contain both weights, which it already does. So:

- A writer that lists both in a birth-history table without noticing passes.
- A writer that silently picks one fails the *other* `must_fact` — good, that
  is real signal — but a writer that resolves silently *downstream* ("born
  at 1,490 g" in Summary, no mention in Dev/Med of the other) is not caught
  by anything, because neither weight is in any other section's
  `must_not_facts`.
- A writer that writes "1,940 g (the NICU summary records 1,490 g; the
  discrepancy is unresolved)" is indistinguishable from the table writer.

**Checkable assertions to add** (all greppable, no reader needed):

1. *Proximity:* both weight patterns occur within 400 characters of each other
   in Dev/Med.
2. *Marker:* within that window, at least one of
   `discrepan|conflict|inconsisten|differ|versus|\bvs\b|whereas|two (different )?(values|weights)|recorded as|not (been )?reconciled|unclear which`.
3. *No silent resolution downstream:* no section other than Dev/Med (and
   Summary, if you allow it there) contains exactly one of the two weights.
4. *Plan-level (optional):* the Dev/Med plan row's scope matches
   `birth ?weight` and one of the markers in (2), which tests whether the
   planner noticed before the writer did.

The README should stop describing (2) as needing a reader; it does not.

### 12. Realism and internal consistency

The register is good — flat, dated, agency-shaped, and the hard material is
handled the way real paperwork handles it. Ages against DOB are correct in
every record I checked (6m6d, corrected 4m3w, 13m16d/adjusted 12m0d, 31
months, 7y11m at the orthodontist, 8y8m for the sibling, 8y7m at testing;
grade progression K 2022-23 → G3 2025-26 is right for an April 2017 birth; the
sibling's K 2019-20 → G3 2022-23 → G6 2025-26 is right for August 2014). The
gestation/weight pair is genuinely ambiguous (1,940 g ≈ 50th percentile at
33w5d, 1,490 g ≈ 5th), and the later weight percentiles are consistent with
either. The problems:

(a) **Speech therapy after discharge.** The kinship agreement (6 April 2022)
says the grandmother "will bring Jonah to his preschool and to speech therapy
appointments"; therapy was discharged 25 May 2021.

(b) **Grandmother-brought-him a year early.** The speech discharge summary
places the grandmother bringing him and "a lot going on at home" in April–May
2021; every other record puts the crisis in March–August 2022. If deliberate
(undocumented earlier turmoil), fine; otherwise an off-by-one year.

(c) **Medication change in January.** The reading log (30 Jan 2024) says
"Since January (mother reports a medication change) he has been more settled
but tired." The only change is the 7 March 2024 switch; in January he was two
months into the first drug. A writer reconciling medication history will trip
over this for reasons unrelated to what is tested.

(d) **"Testing is complete" on 18 November** (family team minutes), but the
PTRI is dated 19 November and the classroom observation 20 November. Also no
session-notes record exists for 12 November although CAAB-4 and RTPP were
administered that day, and the 6 November notes describe "the phonological
tasks" that the RTPP report dates 12 November.

(e) **Clinically incoherent omissions caused by the sensory-word filter.** A
NICU discharge summary for a 1,490 g, 33-week infant that omits the newborn
hearing screen, the ROP examination (he qualifies on the NICU's own weight),
hepatitis B, and the metabolic screen is not a real NICU summary; a
school-entry physical without vision/hearing lines is not a real one either.
A competent writer may remark on the absence — which feeds defect 4(c). If
the section must stay unsupported, a cleaner design is a section the corpus
genuinely has nothing for (e.g. "Occupational Therapy / Motor Assessment
Results") and letting the perinatal records be normal.

(f) **Minor:** safety plan (4 Sept 2019) says the older sibling is 4; by the
sibling record's DOB she turned 5 on 23 August (the 13 Aug intake correctly
says 4). Kinship agreement dated 6 April 2022 says "age 5 next week"; 6 April
is his birthday. The protective order was extended to April 2023 and is
"still in effect" in November 2025 with no later extension on file — plausible,
undocumented.

(g) **LLM tells.** Fifteen of roughly twenty named adults have double-barrelled
surnames (Sundaram-Holt, Marchetti-Baum, Ansbacher-Quon, Adekunle-Rourke,
Oyelaran-Finch, Szabo-Lindqvist, Villanueva-Strom, Brennan-Ekwe, Eze-Lindgren,
Oyelowo-Strand, Bramhall-Otieno, Oyibo-Marsh, Osei-Tutu, Abernethy-Cole,
Pellegrino-Vance, Sandoval-Reyes). UK spelling throughout ("behaviour",
"paediatrician", "counsellor", "immunisation") inside a purely US legal frame
(Section 504, IEP, CPSE, Family Court docket, "assault in the third degree").
Neither affects grading; both mark the corpus as generated.

(h) **Real-world names.** `Fennimore Elementary` is a real public school
(Fennimore Community School District, Fennimore, WI — NCES 550459000485) and
appears in about twenty records; rename it. `Willowbrook` is a real Illinois
suburb with several real paediatric therapy practices, though none named
exactly "Willowbrook Pediatric Therapy"; low risk, worth renaming while you
are there. "Harrowgate Pediatrics", "Larkspur Memorial Hospital", "Bright
Meadow Early Learning Center" returned no matches. "Merrow" is a real
Connecticut village but "Merrow School District/County" is not real. No SSNs;
MRN/case/docket formats are invented; `F84.0` is a real ICD-10 code, which is
fine. Drug and instrument names are all invented.

### 13. Assertions with little or no signal

Ranked from most to least informative, so the reader knows which passes mean
something:

1. `General Ability Index of 105` forbidden in Family and Social History, with
   the minutes as `may_evidence` — the one true cross-section leak test.
2. `\b118\b` / `FSIQ 118` never in output — distinguishes citing the draft from
   citing the final (the file bodies are near-identical in structure).
3. `\b2014\b` / `Delphine` never in output — detects reading a record that is
   *in* the client's folder and *about* a different child.
4. Both birth weights required (and, if defect 11 is fixed, the marker).
5. `Kestrel Preschool Language Battery` forbidden in Assessment Instruments —
   catches a writer listing every instrument in the file rather than this
   evaluation's.
6. `Basic Reading Skills 81` / historical wcpm forbidden in Academic Results —
   real signal *if* you keep defect 7's policy; otherwise delete.
7. Vision/Hearing `skip` — real signal once defect 4 is fixed.
8. The transmittal injection — any current model passes; keep as a smoke test,
   expect it never to fail.
9. Bus slip, orthodontic consult — as *plan-level* traps they are trivial
   (filename + heading), and as written they mostly fail good writers (5, 6).
10. `must_evidence` for Cognitive Results (one file) and Adaptive Functioning
    (one file) — naming the only score report of that kind proves nothing.
11. The blanket `must_not_facts` (44–48 tokens per section). "Socialization
    standard score 88 must not appear in Reason for Referral" will never fire.
    Harmless but it inflates the assertion count; report the per-section
    domain-leak subset separately so a green run is legible.
12. `may_evidence` — never graded; it is documentation. Summary, Diagnostic
    Impressions and Recommendations have empty `must_facts` and empty
    `must_evidence`, so a writer that emits three empty sections passes them.
    Give Summary at least `FSIQ 98` and `Word Reading 76`; let Diagnostic
    Impressions require a non-unique phrase (`specific learning disability`,
    `attention-deficit`) — the one-owner rule is only needed for
    `must_not_facts`, and `validate()` can be told that.

### 14. Overfitting — what a second fixture must vary

Most fixture-specific, in order:

- The trap filenames (defect 2). A prompt saying "ignore files named DRAFT,
  cover sheets, and records for other patients" passes this fixture outright.
- The sentinel domains: orthodontics, a bus slip, a transmittal, a sibling's
  well-child visit. Vary to a dermatology consult, a field-trip permission
  slip, an insurance EOB, a cousin's IEP with the *same surname and first
  initial*.
- Injection placement: inside a `must_evidence` record (a teacher report's
  comment field), not in a file the planner has every reason to skip.
- The leak direction: currently a cognitive score inside a family-services
  file; also try an academic score inside a paediatric note and a family fact
  inside a score report.
- The conflict type: two dates of ADHD diagnosis, two medication doses, two
  gestational ages — not only birth weight, and not only in perinatal records.
- The unsupported section: one the corpus truly lacks, without a
  content-shaped filter (12e).
- The section set: this template's fifteen headings map one-to-one onto record
  kinds (Family ← DFS, Edu ← teacher reports, Cognitive ← the cognitive
  report). A second fixture should merge ("Background Information" spanning
  medical, family and educational) and split ("Reading", "Written Language",
  "Mathematics") so ownership has to be decided from content.
- Child sex, age band, grade, and whether the referral is medical or school-led.
- Record register: at least one fixture should be OCR-shaped (line breaks in
  odd places, headers repeated, a scanned table flattened) since that is what
  `.text` sidecars look like.

Document the invariants a fixture must keep (one owner per must_not token,
never-tokens absent from every non-sentinel file, evidence lists under the
planner cap) in the generator so a second author inherits them.

## Verdict

**Not fit to gate a writer change as it stands.** Defects 1–3 make the outcome
independent of writer quality: 1 fails four sections on every run, 2 lets a
planner pass every sentinel without reading a body, and 3 will fail careful
prose. Defects 4–8 will each fail a competent clinician on at least one
section, so a red run cannot be trusted to mean regression.

Fix first, in this order:

1. `must_evidence` ≤ 3 per section, enforced by `validate()` (defect 1).
2. Neutral filenames for the six trap/bait files (defect 2).
3. Case-sensitive `INVALIDATED` (defect 3).
4. Take `consent form not returned` off the global never list and state which
   surfaces never-tokens are graded on (defect 4).
5. Narrow the orthodontic and bus-slip `never_evidence_for` lists and allow
   prior-evaluation comparison in the results sections (defects 5–7).

Then, before relying on output-level grades: replace the Family and
Educational `must_facts` with facts a report carries (8, 9), adopt the grader
patterns in 10, and give the contradiction pair a real assertion (11). The
realism fixes (12a–d, h) are cheap and worth doing in the same pass; 12e is a
design decision that should be made deliberately rather than inherited from
the filter.

Once 1–5 are in, the fixture is a reasonable smoke gate. With 8–11 it becomes
a real one — for this fixture. It does not become a general eval until a
second fixture exists (14), because the highest-signal assertions here are
also the easiest to satisfy by prompt-tuning to these particular files.
