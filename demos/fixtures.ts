// Demo video fixture data for all three scenarios.

// ── Shared helpers ─────────────────────────────────────────────────────

const ACCOUNT_ID = "185735714230";
const BUCKET_NAME = `${ACCOUNT_ID}-claria-data`;

/** Build a plan entry with the given action/cause. */
function entry(
  resource_type: string,
  resource_name: string,
  label: string,
  description: string,
  severity: string,
  action: string,
  cause: string,
  drift: { field: string; label: string; expected: unknown; actual: unknown }[] = [],
  actual: unknown = null,
) {
  return {
    spec: { resource_type, resource_name, lifecycle: "managed", desired: {}, label, description, severity, iam_actions: [] },
    action,
    cause,
    drift,
    actual,
  };
}

function ok(rt: string, rn: string, label: string, desc: string, sev: string, actual: unknown = null) {
  return entry(rt, rn, label, desc, sev, "ok", "in_sync", [], actual);
}

function create(rt: string, rn: string, label: string, desc: string, sev: string) {
  return entry(rt, rn, label, desc, sev, "create", "first_provision");
}

// ── Full resource list (all in sync) ───────────────────────────────────

export const allOkEntries = [
  ok("iam_user", "claria-admin", "IAM User", "Dedicated least-privilege user that Claria operates as", "info"),
  ok("iam_user_policy", "claria-admin-policy", "IAM Policy", "Permissions scoped to only what Claria needs", "normal"),
  ok("baa_agreement", "aws-baa", "BAA Agreement", "Business Associate Agreement — must be accepted in the AWS Artifact console", "elevated"),
  ok("s3_bucket", BUCKET_NAME, "S3 Bucket", "Encrypted storage for your client records and documents", "normal", { region: "us-east-1" }),
  ok("s3_bucket_versioning", BUCKET_NAME, "S3 Bucket Versioning", "S3 version history — protects against accidental deletion", "normal", { status: "Enabled", mfa_delete: "Enabled" }),
  ok("s3_bucket_encryption", BUCKET_NAME, "S3 Bucket Encryption", "Server-side encryption — your data is encrypted at rest", "normal", { sse_algorithm: "aws:kms", kms_key_id: "arn:aws:kms:us-east-1:185735714230:key/claria-data-key" }),
  ok("s3_bucket_public_access", BUCKET_NAME, "S3 Public Access Block", "All public access blocked — data is private by default", "normal", { block_public_acls: true, block_public_policy: true, ignore_public_acls: true, restrict_public_buckets: true }),
  ok("s3_bucket_policy", BUCKET_NAME, "S3 Bucket Policy", "Enforces TLS-only access to the bucket", "normal"),
  ok("cloudtrail_trail", "claria-audit-trail", "CloudTrail Trail", "Audit log for all S3 data access events", "normal"),
  ok("cloudtrail_s3_events", "claria-audit-trail", "CloudTrail S3 Events", "Data and management event logging for S3 operations", "normal"),
  ok("bedrock_model_access", "anthropic.claude-sonnet-4-20250514-v1:0", "Bedrock Model Access", "Claude Sonnet 4 — enabled for chat", "elevated"),
  ok("bedrock_model_access", "anthropic.claude-haiku-4-5-20251001-v1:0", "Bedrock Model Access", "Claude Haiku 4.5 — enabled for chat", "elevated"),
  ok("bedrock_model_access", "anthropic.claude-opus-4-6-20260301-v1:0", "Bedrock Model Access", "Claude Opus 4.6 — enabled for chat", "elevated"),
];

// ── Scenario 1: Bootstrap (fresh account, all create) ──────────────────

export const freshPlanEntries = [
  create("iam_user", "claria-admin", "IAM User", "Dedicated least-privilege user that Claria operates as", "info"),
  create("iam_user_policy", "claria-admin-policy", "IAM Policy", "Permissions scoped to only what Claria needs", "normal"),
  create("baa_agreement", "aws-baa", "BAA Agreement", "Business Associate Agreement — must be accepted in the AWS Artifact console", "elevated"),
  create("s3_bucket", BUCKET_NAME, "S3 Bucket", "Encrypted storage for your client records and documents", "normal"),
  create("s3_bucket_versioning", BUCKET_NAME, "S3 Bucket Versioning", "S3 version history — protects against accidental deletion", "normal"),
  create("s3_bucket_encryption", BUCKET_NAME, "S3 Bucket Encryption", "Server-side encryption — your data is encrypted at rest", "normal"),
  create("s3_bucket_public_access", BUCKET_NAME, "S3 Public Access Block", "All public access blocked — data is private by default", "normal"),
  create("s3_bucket_policy", BUCKET_NAME, "S3 Bucket Policy", "Enforces TLS-only access to the bucket", "normal"),
  create("cloudtrail_trail", "claria-audit-trail", "CloudTrail Trail", "Audit log for all S3 data access events", "normal"),
  create("cloudtrail_s3_events", "claria-audit-trail", "CloudTrail S3 Events", "Data event logging for object-level S3 operations", "normal"),
  create("bedrock_model_access", "anthropic.claude-sonnet-4-20250514-v1:0", "Bedrock Model Access", "Claude Sonnet 4 — enabled for chat", "elevated"),
  create("bedrock_model_access", "anthropic.claude-haiku-4-5-20251001-v1:0", "Bedrock Model Access", "Claude Haiku 4.5 — enabled for chat", "elevated"),
  create("bedrock_model_access", "anthropic.claude-opus-4-6-20260301-v1:0", "Bedrock Model Access", "Claude Opus 4.6 — enabled for chat", "elevated"),
];

// ── Scenario 2: Sync — policy update drift ─────────────────────────────

export const driftPlanEntries = [
  ok("iam_user", "claria-admin", "IAM User", "Dedicated least-privilege user that Claria operates as", "info"),
  ok("iam_user_policy", "claria-admin-policy", "IAM Policy", "Permissions scoped to only what Claria needs", "normal"),
  ok("baa_agreement", "aws-baa", "BAA Agreement", "Business Associate Agreement — must be accepted in the AWS Artifact console", "elevated"),
  ok("s3_bucket", BUCKET_NAME, "S3 Bucket", "Encrypted storage for your client records and documents", "normal", { region: "us-east-1" }),
  // ---- Drifted resources ----
  entry(
    "s3_bucket_versioning", BUCKET_NAME,
    "S3 Bucket Versioning",
    "S3 version history — now requires MFA Delete for HIPAA compliance",
    "normal",
    "modify", "manifest_changed",
    [{ field: "mfa_delete", label: "MFA Delete", expected: "Enabled", actual: "Disabled" }],
    { status: "Enabled", mfa_delete: "Disabled" },
  ),
  entry(
    "s3_bucket_encryption", BUCKET_NAME,
    "S3 Bucket Encryption",
    "Server-side encryption — upgraded to KMS with dedicated key",
    "normal",
    "modify", "manifest_changed",
    [
      { field: "sse_algorithm", label: "Encryption algorithm", expected: "aws:kms", actual: "AES256" },
      { field: "kms_key_id", label: "KMS key", expected: "arn:aws:kms:us-east-1:185735714230:key/claria-data-key", actual: null },
    ],
    { sse_algorithm: "AES256" },
  ),
  ok("s3_bucket_public_access", BUCKET_NAME, "S3 Public Access Block", "All public access blocked — data is private by default", "normal", { block_public_acls: true, block_public_policy: true, ignore_public_acls: true, restrict_public_buckets: true }),
  ok("s3_bucket_policy", BUCKET_NAME, "S3 Bucket Policy", "Enforces TLS-only access to the bucket", "normal"),
  ok("cloudtrail_trail", "claria-audit-trail", "CloudTrail Trail", "Audit log for all S3 data access events", "normal"),
  entry(
    "cloudtrail_s3_events", "claria-audit-trail",
    "CloudTrail S3 Events",
    "Event logging — expanded to include management events",
    "normal",
    "modify", "manifest_changed",
    [{ field: "include_management_events", label: "Management events", expected: true, actual: false }],
    { data_events: true, management_events: false },
  ),
  ok("bedrock_model_access", "anthropic.claude-sonnet-4-20250514-v1:0", "Bedrock Model Access", "Claude Sonnet 4 — enabled for chat", "elevated"),
  ok("bedrock_model_access", "anthropic.claude-haiku-4-5-20251001-v1:0", "Bedrock Model Access", "Claude Haiku 4.5 — enabled for chat", "elevated"),
  ok("bedrock_model_access", "anthropic.claude-opus-4-6-20260301-v1:0", "Bedrock Model Access", "Claude Opus 4.6 — enabled for chat", "elevated"),
];

// ── Scenario 3: Record + Chat ──────────────────────────────────────────

export const existingClients = [
  { id: "aaaaaaaa-1111-2222-3333-bbbbbbbbbbbb", name: "Jane Doe", created_at: "2026-02-15T10:00:00Z" },
  { id: "cccccccc-4444-5555-6666-dddddddddddd", name: "John Smith", created_at: "2026-02-20T14:30:00Z" },
];

export const caseNotesText = `Alex Rivera — Intake Notes, 3/15/2026
Referral source: School counselor, Ms. Patel

Presenting concerns:
Alex (age 10, 4th grade) was referred for evaluation following a parent-teacher conference where both parties expressed concern about academic performance and behavioral difficulties.

Parent report (Mr. & Mrs. Rivera):
Parents describe Alex as a "smart kid who doesn't apply himself." They report increasing frustration with homework — assignments that should take 20 minutes stretch to over an hour, with frequent breaks and complaints of boredom. Alex's room and backpack are consistently disorganized despite repeated reminders. He loses school supplies weekly. Parents note that Alex excels at building complex LEGO sets and can spend hours on video games, which confuses them about the attention concerns. Increased conflicts with younger sister (age 7) over the past three months — parents describe Alex as "easily set off" and having difficulty calming down after disagreements.

Teacher report (Mr. Okafor, 4th grade):
Alex demonstrates strong verbal participation and creative thinking during class discussions. However, written assignments are frequently incomplete or submitted with minimal effort. Alex struggles with timed math tests despite showing conceptual understanding verbally. He often fidgets with objects at his desk and has difficulty remaining seated during independent work periods. Transitions between subjects are particularly challenging — Alex is typically the last student ready. Mr. Okafor notes that Alex works best in small-group settings with clear structure and immediate feedback.

Previous evaluations: None
Medical history: Unremarkable. No medications.
Developmental milestones: Met on time per parent report.`;

export const chatQuestion = "Based on the intake notes, what are the primary concerns and what assessment battery would you recommend?";

export const chatResponse = `Based on Alex Rivera's intake notes, here is my analysis:

## Primary Concerns

**1. Attention & Executive Function**
The pattern across informants is consistent: difficulty sustaining attention on non-preferred tasks (homework, written work, timed tests), organizational challenges (disorganized backpack/room, losing supplies), and difficulty with transitions. Notably, Alex *can* sustain attention on high-interest activities (LEGO, video games), which is characteristic of ADHD-pattern attention difficulties rather than a global attention deficit.

**2. Written Expression**
Both parents and teacher report a significant gap between Alex's verbal abilities (strong discussion participation, creative thinking) and written output (incomplete, minimal effort). This discrepancy warrants investigation for a possible written expression disorder or whether it is secondary to attention/executive function difficulties.

**3. Emotional Regulation**
Increasing irritability and difficulty calming down after conflicts with his sister, combined with being "easily set off," suggest emerging emotional regulation concerns. These may be secondary to frustration from unaddressed learning/attention needs.

## Recommended Assessment Battery

| Domain | Instrument | Rationale |
|--------|-----------|-----------|
| Cognitive | WISC-V | Establish cognitive profile; examine Working Memory and Processing Speed indices |
| Attention/ADHD | Conners 4 (parent + teacher) | Multi-informant ADHD rating scales with DSM-5 alignment |
| Executive Function | BRIEF-2 (parent + teacher) | Captures real-world executive function in daily settings |
| Achievement | WIAT-4 (selected subtests) | Written Expression, Math Fluency to quantify reported gaps |
| Behavioral/Emotional | BASC-3 (parent + teacher) | Broad screening for internalizing/externalizing concerns |
| Classroom observation | 15-min structured observation | Observe on-task behavior, transitions, and peer interactions |

## Additional Considerations

- Request work samples from school (particularly written assignments) for qualitative comparison
- Consider a brief screener for anxiety (Alex's difficulty with timed tests and "boredom" complaints may reflect avoidance)
- Schedule parent interview to gather more detailed developmental and family history

*Would you like me to draft a consent form and evaluation plan for the family?*`;

// ── Shared config ──────────────────────────────────────────────────────

export const savedConfig = {
  region: "us-east-1",
  system_name: "claria",
  account_id: ACCOUNT_ID,
  created_at: "2026-03-01T17:30:02.048518Z",
  credential_type: "inline",
  profile_name: null,
  access_key_hint: "AKIA...GJEV",
  preferred_model_id: "us.anthropic.claude-opus-4-6-20260301-v1:0",
  cost_explorer_enabled: true,
  hourly_cost_data: false,
};

export const chatModels = [
  { model_id: "us.anthropic.claude-opus-4-6-20260301-v1:0", name: "Claude Opus 4.6" },
  { model_id: "us.anthropic.claude-sonnet-4-20250514-v1:0", name: "Claude Sonnet 4" },
  { model_id: "us.anthropic.claude-haiku-4-5-20251001-v1:0", name: "Claude Haiku 4.5" },
];
