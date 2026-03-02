// Mock IPC responses for screenshot capture.
// Each key is a Tauri command name, each value is the response data.

const CLIENT_ID = "aaaaaaaa-1111-2222-3333-bbbbbbbbbbbb";

/** Shorthand for an in-sync PlanEntry. */
function ok(
  resource_type: string,
  resource_name: string,
  label: string,
  description: string,
  severity: string,
  actual: unknown = null,
) {
  return {
    spec: { resource_type, resource_name, lifecycle: "managed", desired: {}, label, description, severity, iam_actions: [] },
    action: "ok",
    cause: "in_sync",
    drift: [],
    actual,
  };
}

export const fixtures: Record<string, unknown> = {
  has_config: true,

  load_config: {
    region: "us-east-1",
    system_name: "claria",
    account_id: "185735714230",
    created_at: "2026-03-01T17:30:02.048518Z",
    credential_type: "inline",
    profile_name: null,
    access_key_hint: "AKIA...GJEV",
    preferred_model_id: "us.anthropic.claude-opus-4-6-20260301-v1:0",
  },

  list_chat_models: [
    {
      model_id: "us.anthropic.claude-opus-4-6-20260301-v1:0",
      name: "Claude Opus 4.6",
    },
    {
      model_id: "us.anthropic.claude-sonnet-4-20250514-v1:0",
      name: "Claude Sonnet 4",
    },
    {
      model_id: "us.anthropic.claude-haiku-4-5-20251001-v1:0",
      name: "Claude Haiku 4.5",
    },
  ],

  list_clients: [
    {
      id: CLIENT_ID,
      name: "Jane Doe",
      created_at: "2026-02-15T10:00:00Z",
    },
    {
      id: "cccccccc-4444-5555-6666-dddddddddddd",
      name: "John Smith",
      created_at: "2026-02-20T14:30:00Z",
    },
    {
      id: "eeeeeeee-7777-8888-9999-ffffffffffff",
      name: "Maria Garcia",
      created_at: "2026-02-28T09:15:00Z",
    },
  ],

  list_record_files: [
    {
      filename: "intake-parent-interview.txt",
      size: 3200,
      last_modified: "2026-02-15T11:00:00Z",
      is_text: true,
    },
    {
      filename: "teacher-observation.txt",
      size: 2800,
      last_modified: "2026-02-20T15:00:00Z",
      is_text: true,
    },
    {
      filename: "wisc-v-basc-3-results.pdf",
      size: 524288,
      last_modified: "2026-02-18T09:30:00Z",
      is_text: false,
    },
  ],

  list_record_context: [
    {
      filename: "intake-parent-interview.txt",
      text: "Parent interview conducted 2/15/2026. Mother reports difficulty with homework completion, emotional regulation, and peer relationships...",
    },
    {
      filename: "teacher-observation.txt",
      text: "Teacher behavioral checklist and narrative from Ms. Alvarado. Student is frequently off-task, difficulty with transitions, written output below grade level...",
    },
  ],

  "get_prompt:system-prompt": "You are a clinical assistant helping a psychologist set up a new client record. Help gather relevant intake information such as the client's presenting concerns, referral source, relevant history, and initial observations. Be professional, empathetic, and concise. Ask clarifying questions when needed. Do not provide diagnoses or treatment recommendations — your role is to help organize and document the intake information.",

  "get_prompt:pdf-extraction": "Extract the complete text content from this document. Return plain text, preserving paragraph structure. Do not add commentary, headers, or formatting.\n\nPreserve table structure. Use a markdown format.",

  list_prompt_versions: [],

  get_whisper_models: [
    {
      tier: "base_en",
      dir_name: "whisper-base-en",
      label: "Good English",
      description: "English-only, fastest inference",
      download_size: "~293 MB",
      downloaded: true,
      model_size_bytes: 306000000,
      model_path: "/mock/models/whisper-base-en",
      active: false,
      gpu_accelerated: true,
    },
    {
      tier: "small",
      dir_name: "whisper-small",
      label: "Good English + Spanish",
      description: "Multilingual model with good English and Spanish support.",
      download_size: "~967 MB",
      downloaded: false,
      model_size_bytes: null,
      model_path: null,
      active: false,
      gpu_accelerated: false,
    },
    {
      tier: "turbo",
      dir_name: "whisper-large-v3-turbo",
      label: "Best Quality",
      description: "Multilingual, large-v3-turbo",
      download_size: "~1.5 GB",
      downloaded: true,
      model_size_bytes: 1600000000,
      model_path: "/mock/models/whisper-large-v3-turbo",
      active: true,
      gpu_accelerated: true,
    },
  ],

  check_for_updates: {
    current_version: "0.11.0",
    latest_version: "0.11.0",
    update_available: false,
    release_url: "",
  },

  chat_message: {
    chat_id: "demo-chat-0001",
    content: `Here is a developmental and behavioral history compiled from the available records:

## Referral & Presenting Concerns

Jane Doe (age 8, DOB 06/12/2017) was referred for psychoeducational evaluation by her pediatrician, Dr. Reyes, following concerns raised by multiple informants regarding attention, emotional regulation, and academic progress.

**Mother's report (intake notes, 2/15/2026):** Ms. Doe describes Jane as a "bright kid who can't seem to get it together." She reports that homework routinely takes 2–3 hours, with frequent crying and refusal. Jane loses belongings daily and has difficulty following multi-step instructions at home. Ms. Doe also notes increased irritability over the past six months and withdrawal from a longtime friend group.

**Teacher observation (session notes, 2/20/2026):** Jane's 3rd-grade teacher, Ms. Alvarado, completed a behavioral checklist and provided a written narrative. She describes Jane as "eager to participate but often off-task within minutes." Jane frequently leaves her seat, blurts out answers, and struggles to transition between activities. Reading fluency is at grade level, but written output is significantly below expectations. Ms. Alvarado notes that Jane works best in small-group settings with direct prompting.

**Neighbor collateral (intake notes):** A family friend and retired school counselor, Mr. Torres, encouraged the family to pursue testing after observing Jane's difficulty sustaining attention during structured activities and her pattern of emotional outbursts at neighborhood events.

## Assessment Summary

The formal assessment (PDF, 2/18/2026) includes WISC-V, BASC-3 parent and teacher forms, and a Conners 4 rating scale. Full results are available in the client record.

---

*Would you like me to draft a diagnostic summary or begin organizing this into a report template?*`,
  },

  plan: [
    ok("iam_user", "claria-admin", "IAM User", "Dedicated least-privilege user that Claria operates as", "info"),
    ok("iam_user_policy", "claria-admin-policy", "IAM Policy", "Permissions scoped to only what Claria needs", "normal"),
    ok("baa_agreement", "aws-baa", "BAA Agreement", "Business Associate Agreement \u2014 must be accepted in the AWS Artifact console", "elevated"),
    ok("s3_bucket", "185735714230-claria-data", "S3 Bucket", "Encrypted storage for your client records and documents", "normal", { region: "us-east-1" }),
    ok("s3_bucket_versioning", "185735714230-claria-data", "S3 Bucket Versioning", "S3 version history \u2014 protects against accidental deletion", "normal", { status: "Enabled" }),
    ok("s3_bucket_encryption", "185735714230-claria-data", "S3 Bucket Encryption", "Server-side encryption \u2014 your data is encrypted at rest", "normal", { sse_algorithm: "AES256" }),
    ok("s3_bucket_public_access", "185735714230-claria-data", "S3 Public Access Block", "All public access blocked \u2014 data is private by default", "normal", { block_public_acls: true, block_public_policy: true, ignore_public_acls: true, restrict_public_buckets: true }),
    ok("s3_bucket_policy", "185735714230-claria-data", "S3 Bucket Policy", "Enforces TLS-only access to the bucket", "normal", { Version: "2012-10-17", Statement: [{ Effect: "Deny", Principal: "*", Action: "s3:*", Resource: ["arn:aws:s3:::185735714230-claria-data", "arn:aws:s3:::185735714230-claria-data/*"], Condition: { Bool: { "aws:SecureTransport": "false" } } }] }),
    ok("cloudtrail_trail", "claria-audit-trail", "CloudTrail Trail", "Audit log for all S3 data access events", "normal"),
    ok("cloudtrail_s3_events", "claria-audit-trail", "CloudTrail S3 Events", "Data event logging for object-level S3 operations", "normal"),
    ok("bedrock_model_access", "anthropic.claude-sonnet-4-20250514-v1:0", "Bedrock Model Access", "Claude Sonnet 4 \u2014 enabled for chat", "elevated"),
    ok("bedrock_model_access", "anthropic.claude-haiku-4-5-20251001-v1:0", "Bedrock Model Access", "Claude Haiku 4.5 \u2014 enabled for chat", "elevated"),
    ok("bedrock_model_access", "anthropic.claude-opus-4-6-20260301-v1:0", "Bedrock Model Access", "Claude Opus 4.6 \u2014 enabled for chat", "elevated"),
  ],
  transcribe_memo: {
    text: "Session with Jane Doe, March 1st, 2026. Jane presented today with flat affect and limited eye contact. Mother reports increased irritability at home over the past two weeks, coinciding with a change in classroom seating arrangement. Jane was reluctant to engage initially but warmed up during the structured play activity. She demonstrated age-appropriate vocabulary but struggled with narrative sequencing when describing her week. Notable: Jane spontaneously mentioned feeling worried about everything — first unprompted reference to generalized anxiety. Recommend adding GAD-7 child version to next session's battery. Follow up on peer relationship concerns and coordinate with Ms. Alvarado regarding classroom accommodations.",
    language: "en",
  },

  list_deleted_clients: [],
  list_deleted_files: [],
};
