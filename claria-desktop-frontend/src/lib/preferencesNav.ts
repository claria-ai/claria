/**
 * The presentation map of the Preferences screen: which categories exist,
 * which accordion panes live in each, and what search can find inside them.
 *
 * This is pure data — no React, no Tauri. Components register against it via
 * `PANE_COMPONENTS` in `pages/Preferences.tsx`, and every rendered pane wraps
 * itself in a `data-pane` container whose labeled rows carry
 * `data-pref-anchor` attributes matching `SearchableField.anchor`, so a
 * search hit can scroll to and flash the exact control.
 *
 * Nothing here touches the config serialization schema; moving a pane
 * between categories is a data edit, not a migration.
 */

export type CategoryId =
  | "claude"
  | "prompts"
  | "writer"
  | "transcription"
  | "billing";

export type PaneId =
  | "claude.preferred-model"
  | "claude.chat-streaming"
  | "claude.model-tuning"
  | "prompts.chat-system"
  | "prompts.pdf-extraction"
  | "writer.prompt"
  | "writer.whole-report"
  | "writer.prompt-library"
  | "writer.templates"
  | "writer.draft-runs"
  | "writer.limits"
  | "transcription.imported-audio"
  | "transcription.local-models"
  | "transcription.local-compute"
  | "transcription.local-decoding"
  | "billing.cost-explorer";

/** A labeled control search can land on. */
export interface SearchableField {
  /** Matches a `data-pref-anchor` attribute inside the pane. */
  anchor: string;
  /** The visible label, matched by search and shown in results. */
  label: string;
  /** Synonyms that should also find this field. */
  terms?: string[];
}

export interface PaneSpec {
  id: PaneId;
  /** Accordion title. */
  title: string;
  /** One-line description rendered at the top of the pane; searchable. */
  blurb?: string;
  /** Synonyms for the pane as a whole. */
  terms?: string[];
  /** Renders the "This Mac" badge; everything else syncs via S3. */
  machineLocal?: boolean;
  /** Whether the accordion starts open when the category is first shown. */
  defaultOpen?: boolean;
  fields: SearchableField[];
}

export interface CategorySpec {
  id: CategoryId;
  /** Sidebar label and category-page heading. */
  title: string;
  /** Short line under the category heading. */
  blurb: string;
  /** Key into the icon map in Preferences.tsx. */
  icon: "sparkle" | "prompt" | "compose" | "microphone" | "dollar";
  /** Tailwind background class for the sidebar icon tile. */
  tint: string;
  panes: PaneSpec[];
}

export const PREFERENCES_NAV: CategorySpec[] = [
  {
    id: "claude",
    title: "Claude",
    blurb: "Model choice, streaming, and tuning",
    icon: "sparkle",
    tint: "bg-violet-500",
    panes: [
      {
        id: "claude.preferred-model",
        title: "Preferred Model",
        blurb:
          "The model new chat sessions start with. Existing chats keep the model they were started with.",
        terms: ["bedrock", "opus", "sonnet", "haiku", "default model"],
        defaultOpen: true,
        fields: [
          {
            anchor: "preferred_model",
            label: "Preferred model",
            terms: ["model list"],
          },
        ],
      },
      {
        id: "claude.chat-streaming",
        title: "Chat Streaming",
        blurb:
          "How a reply reaches you while the model is writing it. Applies to client chat and infrastructure chat; whatever has already arrived is kept if you stop a reply.",
        terms: ["typing"],
        fields: [
          {
            anchor: "chat_streaming",
            label: "How replies appear",
            terms: ["paragraph", "word by word", "all at once", "stream"],
          },
        ],
      },
      {
        id: "claude.model-tuning",
        title: "Model Tuning",
        blurb:
          "Optional controls applied to chat and writer requests. Each setting is sent only to models that support it — unsupported knobs are skipped automatically, so nothing here can break a model that does not accept them.",
        fields: [
          {
            anchor: "reasoning_enabled",
            label: "Adaptive reasoning",
            terms: ["thinking", "extended thinking"],
          },
          {
            anchor: "effort",
            label: "Effort",
            terms: ["reasoning effort"],
          },
          {
            anchor: "temperature",
            label: "Temperature",
            terms: ["sampling", "randomness"],
          },
        ],
      },
    ],
  },
  {
    id: "prompts",
    title: "Prompts",
    blurb: "The instructions Claria gives Claude",
    icon: "prompt",
    tint: "bg-blue-500",
    panes: [
      {
        id: "prompts.chat-system",
        title: "Chat System Prompt",
        blurb:
          "Instructions given to the AI assistant at the start of every chat session.",
        terms: ["system prompt", "assistant instructions"],
        defaultOpen: true,
        fields: [],
      },
      {
        id: "prompts.pdf-extraction",
        title: "PDF Extraction Prompt",
        blurb:
          "Instructions used when converting uploaded PDF and DOCX files to structured Markdown.",
        terms: ["docx", "markdown", "conversion", "import", "sidecar"],
        fields: [],
      },
    ],
  },
  {
    id: "writer",
    title: "Document Writer",
    blurb: "Prompts, templates, and guardrails for report writing",
    icon: "compose",
    tint: "bg-emerald-600",
    panes: [
      {
        id: "writer.prompt",
        title: "Writer Prompt",
        blurb:
          "Instructions given to the document writer for targeted edits and proposals.",
        terms: ["report", "system prompt", "trust rules", "targeted edits"],
        defaultOpen: true,
        fields: [],
      },
      {
        id: "writer.whole-report",
        title: "Whole-Report Prompt",
        blurb:
          "Instructions used when filling the complete report in one action.",
        terms: ["full draft", "complete report", "trust rules"],
        fields: [],
      },
      {
        id: "writer.prompt-library",
        title: "Prompt Library",
        blurb:
          "Save the writer instructions you reuse — one for each phase of a report. Picking one in a Writing session fills the instruction box, where you can still edit it before sending.",
        terms: ["saved prompts", "steering", "phase", "instruction box"],
        fields: [],
      },
      {
        id: "writer.templates",
        title: "Writer Templates",
        blurb:
          "Keep a small shelf of reusable Word templates in Claria's managed S3 storage. Writing sessions can preview and apply these presets.",
        terms: ["docx", "word", "presets", "upload"],
        fields: [],
      },
      {
        id: "writer.draft-runs",
        title: "Draft runs",
        blurb:
          "How a whole-report draft is planned before it is written, and which models do the supporting work.",
        terms: [
          "plan",
          "gate",
          "planner",
          "reviewer",
          "draft run",
          "sections",
        ],
        fields: [
          {
            anchor: "plan_gate",
            label: "Before drafting starts",
            terms: ["plan", "gate", "review", "auto start", "sections"],
          },
          {
            anchor: "planner_model_id",
            label: "Planning model",
            terms: ["planner", "plan", "sections"],
          },
          {
            anchor: "reviewer_model_id",
            label: "Review model",
            terms: ["reviewer", "review", "findings"],
          },
        ],
      },
      {
        id: "writer.limits",
        title: "Writer Limits",
        blurb:
          "Spend and runtime guardrails for the document writer's agentic loop.",
        terms: ["guardrails", "budget", "runaway"],
        fields: [
          {
            anchor: "max_tool_rounds",
            label: "Tool-use rounds per request",
          },
          {
            anchor: "max_converse_calls",
            label: "Bedrock calls per request",
            terms: ["billed calls", "cost ceiling"],
          },
          {
            anchor: "max_tool_uses_per_response",
            label: "Tool calls per response",
          },
          {
            anchor: "max_retained_turns",
            label: "Conversation turns retained",
            terms: ["context", "history"],
          },
          {
            anchor: "writer_first_frame_timeout_secs",
            label: "Wait for the writer to start responding",
            terms: ["timeout", "slow", "hang", "disconnect", "seconds"],
          },
          {
            anchor: "writer_idle_timeout_secs",
            label: "Wait between writer response chunks",
            terms: ["timeout", "stall", "disconnect", "seconds"],
          },
          {
            anchor: "writer_max_output_tokens",
            label: "Writer response length ceiling",
            terms: ["truncated", "cut short", "max tokens", "length"],
          },
          {
            anchor: "analysis_first_frame_timeout_secs",
            label: "Wait for the planner and reviewer to start responding",
            terms: ["timeout", "planner", "reviewer", "slow", "seconds"],
          },
          {
            anchor: "analysis_idle_timeout_secs",
            label: "Wait between planner and reviewer response chunks",
            terms: ["timeout", "planner", "reviewer", "stall", "seconds"],
          },
        ],
      },
    ],
  },
  {
    id: "transcription",
    title: "Transcription",
    blurb: "Imported audio and on-device memo transcription",
    icon: "microphone",
    tint: "bg-rose-500",
    panes: [
      {
        id: "transcription.imported-audio",
        title: "Imported Audio",
        blurb:
          'Applied to audio files dropped onto a client record. The "Upload audio file…" wizard uses these as starting values and lets you override per file.',
        terms: ["amazon transcribe", "audio upload"],
        defaultOpen: true,
        fields: [
          {
            anchor: "default_language",
            label: "Default language",
            terms: ["english", "spanish", "mixed", "interpreter"],
          },
          {
            anchor: "default_speaker_count",
            label: "Default speakers",
            terms: ["diarization", "speaker count"],
          },
          {
            anchor: "use_medical_for_english",
            label: "Use Transcribe Medical for English sessions",
            terms: ["medical", "clinical vocabulary", "phi"],
          },
          {
            anchor: "translate_to_english",
            label: "Translate non-English segments to English",
            terms: ["translation"],
          },
        ],
      },
      {
        id: "transcription.local-models",
        title: "On-Device Memo Models",
        blurb:
          "Record Memo uses transcribe.cpp and local GGUF models, so microphone audio stays on this computer. Imported audio recordings continue to use Amazon Transcribe.",
        terms: ["whisper", "gguf", "record memo"],
        machineLocal: true,
        defaultOpen: true,
        fields: [
          {
            anchor: "speech_model",
            label: "Memo speech model",
            terms: ["download", "base", "multilingual", "large", "turbo"],
          },
          {
            anchor: "legacy_models",
            label: "Legacy Candle model files",
            terms: ["safetensors", "cleanup"],
          },
        ],
      },
      {
        id: "transcription.local-compute",
        title: "On-Device Compute",
        blurb: "Hardware the local Whisper engine runs on.",
        terms: ["hardware", "gpu", "metal"],
        machineLocal: true,
        fields: [
          {
            anchor: "backend",
            label: "Backend",
            terms: ["metal", "cpu", "gpu"],
          },
          {
            anchor: "gpu_device",
            label: "Compute device index",
          },
          {
            anchor: "cpu_threads",
            label: "CPU threads",
          },
          {
            anchor: "kv_precision",
            label: "K/V cache precision",
            terms: ["f16", "f32", "memory"],
          },
        ],
      },
      {
        id: "transcription.local-decoding",
        title: "Advanced Whisper Decoding",
        blurb:
          "Decoder controls for the local Whisper engine. The defaults suit most recordings.",
        terms: ["decoder"],
        machineLocal: true,
        fields: [
          {
            anchor: "initial_prompt",
            label: "Initial prompt / vocabulary hint",
            terms: ["clinical terms", "names"],
          },
          {
            anchor: "condition_on_previous_text",
            label: "Carry accepted text into each following 30-second window",
            terms: ["context carryover"],
          },
          {
            anchor: "max_previous_context_tokens",
            label: "Previous-context tokens",
          },
          {
            anchor: "temperature",
            label: "Temperature",
          },
          {
            anchor: "temperature_increment",
            label: "Temperature increment",
            terms: ["fallback"],
          },
          {
            anchor: "compression_ratio_threshold",
            label: "Compression-ratio threshold",
            terms: ["repetition"],
          },
          {
            anchor: "log_probability_threshold",
            label: "Log-probability threshold",
          },
          {
            anchor: "no_speech_threshold",
            label: "No-speech threshold",
            terms: ["silence"],
          },
          {
            anchor: "seed",
            label: "Sampling seed",
            terms: ["random", "deterministic"],
          },
        ],
      },
    ],
  },
  {
    id: "billing",
    title: "Billing",
    blurb: "AWS cost reporting",
    icon: "dollar",
    tint: "bg-green-600",
    panes: [
      {
        id: "billing.cost-explorer",
        title: "Cost Explorer",
        blurb:
          "AWS Cost Explorer charges $0.01 per API request. Hourly-resolution data requires separate enablement in the AWS Console and incurs additional storage costs on your AWS bill.",
        terms: ["aws", "spend", "costs"],
        defaultOpen: true,
        fields: [
          {
            anchor: "hourly_cost_data",
            label: "Hourly data resolution",
            terms: ["hourly"],
          },
        ],
      },
    ],
  },
];

/** The Preferences panes a "Manage …" jump from the Writing tab opens. */
export const WRITER_FOCUS_PANES = {
  "writer-templates": "writer.templates",
  "writer-prompts": "writer.prompt-library",
} as const satisfies Record<string, PaneId>;

const PANES_BY_ID = new Map<PaneId, { pane: PaneSpec; category: CategorySpec }>(
  PREFERENCES_NAV.flatMap((category) =>
    category.panes.map((pane) => [pane.id, { pane, category }] as const)
  )
);

export function paneSpec(id: PaneId): PaneSpec {
  const entry = PANES_BY_ID.get(id);
  if (!entry) throw new Error(`Unknown preferences pane: ${id}`);
  return entry.pane;
}

export function categoryOf(id: PaneId): CategoryId {
  const entry = PANES_BY_ID.get(id);
  if (!entry) throw new Error(`Unknown preferences pane: ${id}`);
  return entry.category.id;
}

export function defaultOpenPanes(): PaneId[] {
  return PREFERENCES_NAV.flatMap((category) =>
    category.panes.filter((pane) => pane.defaultOpen).map((pane) => pane.id)
  );
}

// ---------------------------------------------------------------------------
// Search over the static index (titles, labels, blurbs, synonyms)
// ---------------------------------------------------------------------------

export interface PreferenceHit {
  kind: "category" | "pane" | "field";
  categoryId: CategoryId;
  /** null for a category-title hit. */
  paneId: PaneId | null;
  /** Set for field hits; scrolls to the matching `data-pref-anchor`. */
  anchor: string | null;
  /** Display label of what matched. */
  title: string;
  /** Breadcrumb, e.g. "Transcription › Imported Audio". */
  context: string;
}

function matches(query: string, ...texts: (string | undefined)[]): boolean {
  return texts.some((text) => text?.toLowerCase().includes(query));
}

function matchesTerms(query: string, terms?: string[]): boolean {
  return terms?.some((term) => term.toLowerCase().includes(query)) ?? false;
}

/**
 * Case-insensitive substring search over the static index. Results are
 * ordered by match strength (category/pane title, then field label, then
 * synonyms and blurbs), ties broken by navigation order.
 */
export function searchPreferencesNav(rawQuery: string): PreferenceHit[] {
  const query = rawQuery.trim().toLowerCase();
  if (query === "") return [];

  const ranked: { weight: number; hit: PreferenceHit }[] = [];
  for (const category of PREFERENCES_NAV) {
    if (matches(query, category.title)) {
      ranked.push({
        weight: 0,
        hit: {
          kind: "category",
          categoryId: category.id,
          paneId: null,
          anchor: null,
          title: category.title,
          context: category.blurb,
        },
      });
    }
    for (const pane of category.panes) {
      if (matches(query, pane.title)) {
        ranked.push({
          weight: 0,
          hit: paneHit(category, pane),
        });
      } else if (
        matchesTerms(query, pane.terms) ||
        matches(query, pane.blurb)
      ) {
        ranked.push({
          weight: 2,
          hit: paneHit(category, pane),
        });
      }
      for (const field of pane.fields) {
        const labelMatch = matches(query, field.label);
        if (!labelMatch && !matchesTerms(query, field.terms)) continue;
        ranked.push({
          weight: labelMatch ? 1 : 2,
          hit: {
            kind: "field",
            categoryId: category.id,
            paneId: pane.id,
            anchor: field.anchor,
            title: field.label,
            context: `${category.title} › ${pane.title}`,
          },
        });
      }
    }
  }
  // Stable sort: nav order already reflected by push order.
  return ranked.sort((a, b) => a.weight - b.weight).map((entry) => entry.hit);
}

function paneHit(category: CategorySpec, pane: PaneSpec): PreferenceHit {
  return {
    kind: "pane",
    categoryId: category.id,
    paneId: pane.id,
    anchor: null,
    title: pane.title,
    context: category.title,
  };
}
