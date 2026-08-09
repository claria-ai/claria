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
    spec: { resource_type, resource_name, lifecycle: "managed", desired: {}, credential_scope: "regular", label, description, severity, iam_actions: [] },
    action: "ok",
    cause: "in_sync",
    drift: [],
    actual,
  };
}

const planEntries = [
  ok("iam_user", "claria-admin", "IAM User", "Dedicated least-privilege user that Claria operates as", "info"),
  ok("iam_user_policy", "claria-admin-policy", "IAM Policy", "Permissions scoped to only what Claria needs", "normal"),
  ok("baa_agreement", "aws-baa", "BAA Agreement", "Business Associate Agreement — must be accepted in the AWS Artifact console", "elevated"),
  ok("s3_bucket", "185735714230-claria-data", "S3 Bucket", "Encrypted storage for your client records and documents", "normal", { region: "us-east-1" }),
  ok("s3_bucket_versioning", "185735714230-claria-data", "S3 Bucket Versioning", "S3 version history — protects against accidental deletion", "normal", { status: "Enabled" }),
  ok("s3_bucket_encryption", "185735714230-claria-data", "S3 Bucket Encryption", "Server-side encryption — your data is encrypted at rest", "normal", { sse_algorithm: "AES256" }),
  ok("s3_bucket_public_access", "185735714230-claria-data", "S3 Public Access Block", "All public access blocked — data is private by default", "normal", { block_public_acls: true, block_public_policy: true, ignore_public_acls: true, restrict_public_buckets: true }),
  ok("s3_bucket_policy", "185735714230-claria-data", "S3 Bucket Policy", "Enforces TLS-only access to the bucket", "normal", { Version: "2012-10-17", Statement: [{ Effect: "Deny", Principal: "*", Action: "s3:*", Resource: ["arn:aws:s3:::185735714230-claria-data", "arn:aws:s3:::185735714230-claria-data/*"], Condition: { Bool: { "aws:SecureTransport": "false" } } }] }),
  ok("cloudtrail_trail", "claria-audit-trail", "CloudTrail Trail", "Audit log for all S3 data access events", "normal"),
  ok("cloudtrail_s3_events", "claria-audit-trail", "CloudTrail S3 Events", "Data event logging for object-level S3 operations", "normal"),
  ok("bedrock_model_access", "anthropic.claude-sonnet-4-20250514-v1:0", "Bedrock Model Access", "Claude Sonnet 4 — enabled for chat", "elevated"),
  ok("bedrock_model_access", "anthropic.claude-haiku-4-5-20251001-v1:0", "Bedrock Model Access", "Claude Haiku 4.5 — enabled for chat", "elevated"),
  ok("bedrock_model_access", "anthropic.claude-opus-4-6-20260301-v1:0", "Bedrock Model Access", "Claude Opus 4.6 — enabled for chat", "elevated"),
];

/**
 * Same plan with drift: IAM policy modified (elevated scope, so the
 * escalation notice renders), one Bedrock model missing.
 */
export const driftedPlan = planEntries.map((e) => {
  if (e.spec.resource_type === "iam_user_policy") {
    return {
      ...e,
      spec: { ...e.spec, credential_scope: "elevated" },
      action: "modify",
      cause: "drift",
      drift: [
        {
          field: "actions",
          label: "Allowed IAM actions",
          expected: [
            "bedrock:InvokeModel",
            "bedrock:InvokeModelWithResponseStream",
            "ce:GetCostAndUsage",
            "s3:GetObject",
            "s3:PutObject",
            "transcribe:StartTranscriptionJob",
          ],
          actual: [
            "bedrock:GetUseCaseForModelAccess",
            "bedrock:InvokeModel",
            "s3:GetObject",
            "s3:PutObject",
            "transcribe:StartTranscriptionJob",
          ],
        },
      ],
    };
  }
  if (e.spec.resource_name === "anthropic.claude-opus-4-6-20260301-v1:0") {
    return { ...e, action: "create", cause: "missing", actual: null };
  }
  return e;
});

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
    cost_explorer_enabled: true,
    hourly_cost_data: false,
    prompt_caching_enabled: true,
    transcription: {
      default_language: "english",
      default_speaker_count: 2,
      use_medical_for_english: false,
      translate_to_english: false,
    },
  },

  // `fetch_cloud_preferences` returns the same shape as `load_config` —
  // the in-S3 synced preferences mirror the synced subset of the local config.
  fetch_cloud_preferences: {
    region: "us-east-1",
    system_name: "claria",
    account_id: "185735714230",
    created_at: "2026-03-01T17:30:02.048518Z",
    credential_type: "inline",
    profile_name: null,
    access_key_hint: "AKIA...GJEV",
    preferred_model_id: "us.anthropic.claude-opus-4-6-20260301-v1:0",
    cost_explorer_enabled: true,
    hourly_cost_data: false,
    prompt_caching_enabled: true,
    transcription: {
      default_language: "english",
      default_speaker_count: 2,
      use_medical_for_english: false,
      translate_to_english: false,
    },
  },

  save_preferences: null,
  save_transcript_edits: null,
  upload_record_file_with_options: {
    filename: "session-2026-03-15.m4a",
    size: 4_823_521,
    last_modified: new Date().toISOString(),
    is_text: false,
  },
  pick_audio_file: "/Users/clinician/Documents/visit-2026-03-15.m4a",

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

  get_client_record_details: {
    id: CLIENT_ID,
    name: "Jane Doe",
    created_at: "2026-02-15T10:00:00Z",
    updated_at: "2026-03-15T15:08:05Z",
    file_count: 4,
    storage_bytes: 6291456,
    storage_bytes_with_history: 9437184,
    name_history: [
      { name: "Jane Doe", changed_at: "2026-03-15T15:08:05Z" },
      { name: "Jane Marie Doe", changed_at: "2026-03-01T09:30:00Z" },
      { name: "Jane Doe", changed_at: "2026-02-15T10:00:00Z" },
    ],
  },

  update_client_name: {
    id: CLIENT_ID,
    name: "Jane Doe",
    updated_at: "2026-03-15T15:08:05Z",
  },

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
    {
      filename: "session-2026-03-15.m4a",
      size: 4_823_521,
      last_modified: "2026-03-15T14:22:00Z",
      is_text: false,
    },
    {
      filename: "chat-history/cccccccc-4444-5555-6666-dddddddddddd.json",
      size: 9400,
      last_modified: "2026-03-02T10:15:00Z",
      is_text: true,
    },
  ],

  load_chat_history: {
    chat_id: "cccccccc-4444-5555-6666-dddddddddddd",
    model_id: "us.anthropic.claude-opus-4-6-20260301-v1:0",
    created_at: "2026-03-02T10:15:00Z",
    messages: [
      {
        role: "user",
        content: "Summarize the teacher observation notes.",
        usage: null,
      },
      {
        role: "assistant",
        content:
          "Ms. Alvarado describes Jane as eager to participate but often off-task within minutes — frequent out-of-seat behavior, blurting, and difficulty with transitions. Reading fluency is at grade level while written output is significantly below expectations; she works best in small groups with direct prompting.",
        usage: {
          model_id: "us.anthropic.claude-opus-4-6-20260301-v1:0",
          input_tokens: 5120,
          output_tokens: 96,
          cache_read_input_tokens: 0,
          cache_write_input_tokens: 0,
          cost_usd: 0.0842,
          pricing_version: 3,
        },
      },
    ],
  },

  // Sample headered transcript body for the transcript-editor screenshot.
  // Routed by filename via `cmd:filename` (see tauri-mock.ts).
  "get_record_file_text:session-2026-03-15.m4a": [
    "[Clinician 00:00– 00:04]",
    "How are you feeling today?",
    "",
    "[Patient 00:04– 00:09]",
    "I've been having headaches for about a week.",
    "",
    "[Clinician 00:09– 00:12 es-US]",
    "¿En qué parte de la cabeza?",
    "> What part of your head?",
    "",
    "[Patient 00:12– 00:17 es-US]",
    "Sobre todo aquí, en las sienes. A veces también detrás de los ojos.",
    "> Mostly here, at the temples. Sometimes also behind the eyes.",
    "",
    "[Clinician 00:17– 00:22]",
    "Have you noticed anything that makes them worse — screens, lack of sleep, stress?",
  ].join("\n").replace(/– /g, "–"),

  load_report_workspace: {
    schema_version: 2,
    report_id: "99999999-9999-4999-8999-999999999999",
    client_id: CLIENT_ID,
    draft: {
      revision: 3,
      content: {
        title: "Psychological Evaluation",
        sections: [
          {
            id: "11111111-1111-4111-8111-111111111111",
            heading: "Reason for Referral",
            blocks: [
              {
                kind: "paragraph",
                text: "Jane was referred for evaluation of persistent attention, emotional-regulation, and written-output concerns across home and school settings.",
              },
            ],
          },
          {
            id: "22222222-2222-4222-8222-222222222222",
            heading: "Background",
            blocks: [
              {
                kind: "paragraph",
                text: "Parent and teacher interviews describe age-appropriate reading fluency alongside difficulty sustaining effort during independent written work.",
              },
              {
                kind: "bullet_list",
                items: [
                  "Frequent redirection during transitions",
                  "Stronger performance in small-group instruction",
                ],
              },
            ],
          },
        ],
      },
      created_at: "2026-03-01T17:30:02Z",
      updated_at: "2026-03-15T15:04:00Z",
      last_applied_proposal_id: "77777777-7777-4777-8777-777777777777",
    },
    turns: [
      {
        id: "88888888-8888-4888-8888-888888888888",
        model_id: "us.anthropic.claude-opus-4-6-20260301-v1:0",
        timeline: [
          {
            kind: "message",
            role: "user",
            text: "Review the intake and teacher observation, then propose a concise behavioral findings section.",
            created_at: "2026-03-15T15:08:00Z",
          },
          {
            kind: "tool_activity",
            name: "list_record_files",
            summary: "Listed 4 record files",
            status: "succeeded",
            invocation_json: JSON.stringify({ toolUse: { toolUseId: "list-1", name: "list_record_files", input: {} } }, null, 2),
            result_json: JSON.stringify({ toolResult: { toolUseId: "list-1", status: "success", content: [{ json: { file_count: 4, truncated: false } }] } }, null, 2),
            created_at: "2026-03-15T15:08:01Z",
          },
          {
            kind: "tool_activity",
            name: "read_record_file",
            summary: "Read teacher-observation.txt, characters 0–2800",
            status: "succeeded",
            invocation_json: JSON.stringify({ toolUse: { toolUseId: "read-1", name: "read_record_file", input: { filename: "teacher-observation.txt", offset: 0, limit: 8000 } } }, null, 2),
            result_json: JSON.stringify({ toolResult: { toolUseId: "read-1", status: "success", content: [{ json: { filename: "teacher-observation.txt", offset: 0, returned_characters: 2800, total_characters: 2800, content_retained: false } }] } }, null, 2),
            created_at: "2026-03-15T15:08:02Z",
          },
          {
            kind: "tool_activity",
            name: "propose_report_changes",
            summary: "Staged report changes for approval",
            status: "succeeded",
            invocation_json: JSON.stringify({ toolUse: { toolUseId: "proposal-1", name: "propose_report_changes", input: { summary: "Add behavioral findings", operations: [{ kind: "add_section", position: 2, heading: "Behavioral Findings", blocks: [{ kind: "paragraph", text: "Across informants, Jane demonstrates difficulty sustaining attention." }] }] } } }, null, 2),
            result_json: JSON.stringify({ toolResult: { toolUseId: "proposal-1", status: "success", content: [{ json: { status: "pending_user_acceptance", proposal_id: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee" } }] } }, null, 2),
            created_at: "2026-03-15T15:08:04Z",
          },
          {
            kind: "message",
            role: "assistant",
            text: "I staged a focused findings section for your review.",
            created_at: "2026-03-15T15:08:05Z",
          },
        ],
        usage: {
          model_id: "us.anthropic.claude-opus-4-6-20260301-v1:0",
          input_tokens: 4860,
          output_tokens: 410,
          cache_read_input_tokens: 0,
          cache_write_input_tokens: 0,
          cost_usd: 0.03455,
          pricing_version: 4,
        },
        usage_complete: true,
        converse_calls: 3,
        tool_uses: 3,
        context_reads: [
          {
            filename: "teacher-observation.txt",
            offset: 0,
            returned_characters: 2800,
            total_characters: 2800,
            read_at: "2026-03-15T15:08:02Z",
          },
        ],
        created_at: "2026-03-15T15:08:00Z",
        completed_at: "2026-03-15T15:08:05Z",
      },
    ],
    pending_proposal: {
      id: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
      report_id: "99999999-9999-4999-8999-999999999999",
      base_revision: 3,
      model_id: "us.anthropic.claude-opus-4-6-20260301-v1:0",
      summary: "Add behavioral findings from the reviewed records",
      operations: [
        {
          kind: "add_section",
          position: 2,
          section: {
            id: "33333333-3333-4333-8333-333333333333",
            heading: "Behavioral Findings",
            blocks: [
              {
                kind: "paragraph",
                text: "Across informants, Jane demonstrates difficulty sustaining attention and organizing written responses, with improvement under direct prompting.",
              },
              {
                kind: "bullet_list",
                items: [
                  "Leaves her seat during independent work",
                  "Benefits from predictable transitions and brief check-ins",
                ],
              },
              {
                kind: "table",
                rows: [
                  ["Setting", "Observed support"],
                  ["Classroom", "Brief check-ins"],
                  ["Transitions", "Predictable sequence"],
                ],
                has_header: true,
                column_widths: [3500, 6500],
              },
            ],
          },
        },
      ],
      proposed_content: {
        title: "Psychological Evaluation",
        sections: [
          {
            id: "11111111-1111-4111-8111-111111111111",
            heading: "Reason for Referral",
            blocks: [{
              kind: "paragraph",
              text: "Jane was referred for evaluation of persistent attention, emotional-regulation, and written-output concerns across home and school settings.",
            }],
          },
          {
            id: "22222222-2222-4222-8222-222222222222",
            heading: "Background",
            blocks: [
              {
                kind: "paragraph",
                text: "Parent and teacher interviews describe age-appropriate reading fluency alongside difficulty sustaining effort during independent written work.",
              },
              {
                kind: "bullet_list",
                items: [
                  "Frequent redirection during transitions",
                  "Stronger performance in small-group instruction",
                ],
              },
            ],
          },
          {
            id: "33333333-3333-4333-8333-333333333333",
            heading: "Behavioral Findings",
            blocks: [
              {
                kind: "paragraph",
                text: "Across informants, Jane demonstrates difficulty sustaining attention and organizing written responses, with improvement under direct prompting.",
              },
              {
                kind: "bullet_list",
                items: [
                  "Leaves her seat during independent work",
                  "Benefits from predictable transitions and brief check-ins",
                ],
              },
              {
                kind: "table",
                rows: [
                  ["Setting", "Observed support"],
                  ["Classroom", "Brief check-ins"],
                  ["Transitions", "Predictable sequence"],
                ],
                has_header: true,
                column_widths: [3500, 6500],
              },
            ],
          },
        ],
      },
      created_at: "2026-03-15T15:08:04Z",
    },
    resolutions: [],
    last_agent_revision: 3,
    last_export: {
      revision: 2,
      status: "exported",
      attempted_at: "2026-03-14T12:00:00Z",
    },
    template_import: null,
    created_at: "2026-03-01T17:30:02Z",
    updated_at: "2026-03-15T15:08:05Z",
  },
  list_editor_history: [
    {
      report_id: "99999999-9999-4999-8999-999999999999",
      title: "Psychological Evaluation",
      revision: 3,
      turn_count: 1,
      updated_at: "2026-03-15T15:08:05Z",
      last_export: {
        revision: 2,
        status: "exported",
        attempted_at: "2026-03-14T12:00:00Z",
      },
    },
  ],
  export_report_docx: {
    exported: false,
    report_id: "99999999-9999-4999-8999-999999999999",
    revision: 3,
    status: "canceled",
    attempted_at: "2026-03-15T16:00:00Z",
    status_persisted: true,
  },

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

  get_local_transcription_status: {
    runtime_version: "0.2.0",
    accelerated: true,
    legacy_model_bytes: 0,
    settings: {
      settings_version: 1,
      speech_model: "whisper_turbo_q8",
      backend: "auto",
      gpu_device: 0,
      cpu_threads: 0,
      kv_precision: "auto",
      initial_prompt: "",
      condition_on_previous_text: true,
      max_previous_context_tokens: 223,
      temperature: 0,
      temperature_increment: 0.2,
      compression_ratio_threshold: 2.4,
      log_probability_threshold: -1,
      no_speech_threshold: 0.6,
      seed: 0,
    },
    backends: [
      { backend: "auto", label: "Automatic", available: true },
      { backend: "cpu", label: "CPU", available: true },
      { backend: "metal", label: "Metal", available: true },
    ],
    devices: [
      {
        name: "Metal",
        description: "Apple M4 Pro",
        kind: "metal",
        device_type: "igpu",
        device_id: null,
        memory_total: 25769803776,
        memory_free: 21474836480,
        index: 1,
      },
    ],
    models: [
      {
        id: "whisper_base_en_q8",
        label: "Whisper Base English",
        description: "Fast, compact English-only speech model for live memos.",
        filename: "whisper-base.en-Q8_0.gguf",
        quantization: "Q8_0",
        languages: ["en"],
        download_size_bytes: 84886208,
        downloaded: true,
        model_size_bytes: 84886208,
        model_path: "/mock/models/transcribe-cpp/whisper-base.en-Q8_0.gguf",
        active: false,
      },
      {
        id: "whisper_small_q8",
        label: "Whisper Small Multilingual",
        description: "Balanced local model with English, Spanish, and 97 more languages.",
        filename: "whisper-small-Q8_0.gguf",
        quantization: "Q8_0",
        languages: ["multilingual", "en", "es"],
        download_size_bytes: 269751136,
        downloaded: false,
        model_size_bytes: null,
        model_path: null,
        active: false,
      },
      {
        id: "whisper_turbo_q8",
        label: "Whisper Large v3 Turbo",
        description: "Highest-quality curated Whisper model for multilingual transcription.",
        filename: "whisper-large-v3-turbo-Q8_0.gguf",
        quantization: "Q8_0",
        languages: ["multilingual", "en", "es"],
        download_size_bytes: 886381760,
        downloaded: true,
        model_size_bytes: 886381760,
        model_path: "/mock/models/transcribe-cpp/whisper-large-v3-turbo-Q8_0.gguf",
        active: true,
      },
    ],
  },
  save_local_transcription_settings: null,
  delete_local_model: null,
  delete_legacy_transcription_models: null,

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

  plan: planEntries,
  transcribe_memo: {
    text: "Session with Jane Doe, March 1st, 2026. Jane presented today with flat affect and limited eye contact. Mother reports increased irritability at home over the past two weeks, coinciding with a change in classroom seating arrangement. Jane was reluctant to engage initially but warmed up during the structured play activity. She demonstrated age-appropriate vocabulary but struggled with narrative sequencing when describing her week. Notable: Jane spontaneously mentioned feeling worried about everything — first unprompted reference to generalized anxiety. Recommend adding GAD-7 child version to next session's battery. Follow up on peer relationship concerns and coordinate with Ms. Alvarado regarding classroom accommodations.",
    language: "en",
    model_id: "whisper_turbo_q8",
    backend: "MTL0",
  },

  infra_chat: {
    usage: {
      model_id: "us.anthropic.claude-opus-4-6-20260301-v1:0",
      input_tokens: 6242,
      output_tokens: 388,
      cache_read_input_tokens: 0,
      cache_write_input_tokens: 0,
      cost_usd: 0.1227,
      pricing_version: 3,
    },
    content: `Your data is well protected. Here's a summary of the security configuration for account **185735714230**:

## Encryption

- **At rest:** Your S3 bucket \`185735714230-claria-data\` uses AES-256 server-side encryption. Every object is encrypted before it's written to disk.
- **In transit:** The bucket policy enforces TLS — any request without \`aws:SecureTransport\` is denied, so data can never travel unencrypted.

## Access Control

- **IAM user:** Claria operates as \`claria-admin\`, a dedicated least-privilege IAM user. The attached policy (\`claria-admin-policy\`) grants only the specific S3, Bedrock, CloudTrail, and Transcribe actions Claria needs — nothing more.
- **Public access:** All four public-access-block settings are enabled on the bucket (block public ACLs, block public policy, ignore public ACLs, restrict public buckets). There is no way to accidentally expose data.

## Audit Trail

- **CloudTrail** trail \`claria-audit-trail\` is active and logging S3 data events. Every \`GetObject\`, \`PutObject\`, and \`DeleteObject\` call is recorded, giving you a complete audit log for HIPAA compliance.

## Versioning & Recovery

- S3 versioning is **enabled**, so deleted or overwritten files can be recovered from previous versions. Claria's restore flow creates new versions rather than removing delete markers, preserving the full history.

## BAA

- The AWS Business Associate Agreement is in place, covering S3, Bedrock, CloudTrail, and Transcribe under HIPAA.

All 14 resources are currently **in sync** — no drift detected.`,
  },

  get_cost_and_usage: { periods: generateCostData() },

  count_client_context_tokens: 2247,
  count_infra_context_tokens: 8530,

  list_deleted_clients: [],
  list_deleted_files: [],

  list_file_versions: [
    { version_id: "ver-20260303-1600", size: 3200, last_modified: "2026-03-03T16:00:00Z", is_latest: true },
    { version_id: "ver-20260301-1030", size: 2800, last_modified: "2026-03-01T10:30:00Z", is_latest: false },
    { version_id: "ver-20260225-1415", size: 2200, last_modified: "2026-02-25T14:15:00Z", is_latest: false },
    { version_id: "ver-20260215-1100", size: 1600, last_modified: "2026-02-15T11:00:00Z", is_latest: false },
  ],

  "get_file_version_text:ver-20260303-1600":
    "Jane Doe \u2014 Parent Interview, 2/15/2026\nReferral: Dr. Reyes (pediatrician)\nHomework takes 2-3 hours with frequent crying and refusal.\nLoses belongings daily. Difficulty with multi-step instructions.\nIncreased irritability over past six months.\nCollateral: Mr. Torres encouraged formal testing.",

  "get_file_version_text:ver-20260301-1030":
    "Jane Doe \u2014 Parent Interview, 2/15/2026\nReferral: Dr. Reyes (pediatrician)\nHomework takes 2-3 hours, with frequent crying and refusal.\nLoses belongings daily. Difficulty with multi-step instructions.\nIncreased irritability over past six months.",

  "get_file_version_text:ver-20260225-1415":
    "Jane Doe \u2014 Parent Interview, 2/15/2026\nHomework takes 2-3 hours, with frequent crying and refusal.\nLoses belongings daily. Difficulty with multi-step instructions.",

  "get_file_version_text:ver-20260215-1100":
    "Jane Doe \u2014 Parent Interview, 2/15/2026\nHomework takes 2-3 hours, with frequent crying and refusal.",
};

/** Generate 30 days of realistic cost data totaling ~$8. */
function generateCostData() {
  const periods = [];
  // Daily costs per service (base + random variance)
  const services = [
    { key: "Amazon Bedrock", base: 0.15, variance: 0.12 },
    { key: "Amazon Simple Storage Service", base: 0.035, variance: 0.01 },
    { key: "AWS CloudTrail", base: 0.04, variance: 0.015 },
    { key: "AWS Cost Explorer", base: 0.01, variance: 0.01 },
  ];
  // Seed a deterministic pseudo-random sequence
  let seed = 42;
  function rand() {
    seed = (seed * 16807 + 0) % 2147483647;
    return (seed - 1) / 2147483646;
  }
  for (let i = 29; i >= 0; i--) {
    const d = new Date(2026, 2, 3); // March 3
    d.setDate(d.getDate() - i);
    const start = d.toISOString().slice(0, 10);
    const next = new Date(d);
    next.setDate(next.getDate() + 1);
    const end = next.toISOString().slice(0, 10);
    // Weekend days have lower Bedrock usage
    const dayOfWeek = d.getDay();
    const weekendFactor = dayOfWeek === 0 || dayOfWeek === 6 ? 0.3 : 1.0;
    const groups = services.map((s) => {
      const factor = s.key === "Amazon Bedrock" ? weekendFactor : 1.0;
      const amount = Math.max(0, (s.base + (rand() - 0.4) * s.variance) * factor);
      return { key: s.key, amount: amount.toFixed(4), unit: "USD" };
    });
    periods.push({ start, end, groups });
  }
  return periods;
}
