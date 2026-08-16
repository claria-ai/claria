import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

const configInfo = {
  preferred_model_id: null,
  hourly_cost_data: false,
  chat_streaming: "paragraph",
  transcription: {
    default_language: "english",
    default_speaker_count: 2,
    use_medical_for_english: false,
    translate_to_english: false,
  },
  report_authoring: {
    max_tool_rounds: 40,
    max_converse_calls: 50,
    max_tool_uses_per_response: 80,
    max_retained_turns: 200,
  },
  model_tuning: { reasoning_enabled: false, effort: null, temperature: null },
  draft_pipeline: {
    plan_gate: "gated",
    planner_model_id: null,
    reviewer_model_id: null,
  },
};

const localStatus = {
  runtime_version: "0.2.0",
  accelerated: true,
  legacy_model_bytes: 0,
  devices: [
    {
      index: 0,
      kind: "metal",
      name: "Apple M4",
      description: "Apple M4 Pro",
      memory_total: 0,
    },
  ],
  backends: [{ backend: "auto", label: "Automatic", available: true }],
  models: [
    {
      id: "base_en",
      label: "Whisper Base English",
      description: "Fast English-only model.",
      quantization: "Q8_0",
      download_size_bytes: 81_000_000,
      languages: ["en"],
      downloaded: true,
      active: true,
      model_path: "/models/base.gguf",
    },
  ],
  settings: {
    speech_model: "base_en",
    backend: "auto",
    gpu_device: 0,
    cpu_threads: 0,
    kv_precision: "auto",
    initial_prompt: "",
    condition_on_previous_text: true,
    max_previous_context_tokens: 224,
    temperature: 0,
    temperature_increment: 0.2,
    compression_ratio_threshold: 2.4,
    log_probability_threshold: -1,
    no_speech_threshold: 0.6,
    seed: 0,
  },
};

vi.mock("../lib/logBridge", () => ({ logFrontendEvent: vi.fn() }));
vi.mock("../lib/tauri", () => ({
  getPrompt: vi.fn(async (name: string) =>
    name === "system-prompt"
      ? "You are a clinical assistant helping a psychologist."
      : `${name} body`
  ),
  getWriterTrustRules: vi.fn(async () => ({
    targeted: "targeted trust rules",
    full_draft: "full-draft trust rules",
  })),
  savePrompt: vi.fn(async () => {}),
  deletePrompt: vi.fn(async () => {}),
  setPreferredModel: vi.fn(async () => {}),
  getLocalTranscriptionStatus: vi.fn(async () => localStatus),
  downloadLocalModel: vi.fn(async () => localStatus),
  deleteLocalModel: vi.fn(async () => localStatus),
  deleteLegacyTranscriptionModels: vi.fn(async () => localStatus),
  saveLocalTranscriptionSettings: vi.fn(async () => localStatus),
  loadConfig: vi.fn(async () => configInfo),
  setHourlyCostData: vi.fn(async () => {}),
  getCostAndUsage: vi.fn(async () => ({})),
  savePreferencesPatch: vi.fn(async (patch: object) => ({
    ...configInfo,
    ...patch,
  })),
  fetchCloudPreferences: vi.fn(async () => configInfo),
  uploadWriterTemplate: vi.fn(async () => null),
  renameWriterTemplate: vi.fn(async () => ({})),
  deleteWriterTemplate: vi.fn(async () => {}),
  saveWriterLibraryPrompt: vi.fn(async () => ({})),
  deleteWriterLibraryPrompt: vi.fn(async () => {}),
  exportPreferences: vi.fn(async () => true),
  importPreferences: vi.fn(async () => configInfo),
  listPreferencesVersions: vi.fn(async () => []),
  getPreferencesVersion: vi.fn(async () => "{}"),
  restorePreferencesVersion: vi.fn(async () => {}),
  listWriterTemplates: vi.fn(async () => []),
  listWriterLibraryPrompts: vi.fn(async () => [
    { id: "p1", name: "Phase 1", body: "Fill in the Reason for Referral." },
  ]),
  // Reached through lib/versions (VersionHistoryModal sources).
  listPromptVersions: vi.fn(async () => []),
  getPromptVersion: vi.fn(async () => ""),
  restorePromptVersion: vi.fn(async () => {}),
  listFileVersions: vi.fn(async () => []),
  getFileVersionText: vi.fn(async () => ""),
  restoreFileVersion: vi.fn(async () => {}),
}));

import Preferences from "./Preferences";
import { savePreferencesPatch } from "../lib/tauri";
import { PREFERENCES_NAV } from "../lib/preferencesNav";
import { ChatModelsContext, type ChatModelsState } from "../lib/chatModels";

const modelsState: ChatModelsState = {
  models: [
    { model_id: "us.sonnet", name: "Claude Sonnet" },
    { model_id: "us.haiku", name: "Claude Haiku" },
  ],
  loading: false,
  error: null,
  preferredModelId: null,
  retry: () => {},
  setPreferredModelId: () => {},
};

function renderWithModels() {
  return render(
    <ChatModelsContext.Provider value={modelsState}>
      <Preferences navigate={vi.fn()} />
    </ChatModelsContext.Provider>
  );
}

function categoryVisible(paneOrHeading: Element): boolean {
  return paneOrHeading.closest("div[hidden]") === null;
}

function paneElement(paneId: string): Element {
  const pane = document.querySelector(`[data-pane="${paneId}"]`);
  if (!pane) throw new Error(`pane ${paneId} not mounted`);
  return pane;
}

describe("Preferences", () => {
  it("mounts a pane for every entry in the nav map", async () => {
    render(<Preferences navigate={vi.fn()} />);
    await screen.findByText("Preferred Model");
    for (const category of PREFERENCES_NAV) {
      for (const pane of category.panes) {
        expect(document.querySelector(`[data-pane="${pane.id}"]`)).toBeTruthy();
      }
    }
  });

  it("lists every category in the sidebar and lands on Claude", async () => {
    render(<Preferences navigate={vi.fn()} />);
    for (const category of PREFERENCES_NAV) {
      expect(
        screen.getByRole("button", { name: category.title })
      ).toBeTruthy();
    }
    await screen.findByText("Preferred Model");
    expect(categoryVisible(paneElement("claude.preferred-model"))).toBe(true);
    expect(categoryVisible(paneElement("writer.templates"))).toBe(false);
  });

  it("switches categories from the sidebar", async () => {
    const user = userEvent.setup();
    render(<Preferences navigate={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Document Writer" }));
    expect(categoryVisible(paneElement("writer.templates"))).toBe(true);
    expect(categoryVisible(paneElement("claude.preferred-model"))).toBe(false);
  });

  it("search reveals a match: category selected and pane forced open", async () => {
    const user = userEvent.setup();
    render(<Preferences navigate={vi.fn()} />);

    await user.type(screen.getByLabelText("Search settings"), "adaptive");
    // The static index resolves the synonym-free label immediately.
    const results = await screen.findByTestId("pref-search-results");
    await user.click(
      within(results).getByText("Adaptive reasoning").closest("button")!
    );

    // Model Tuning is not open by default; the hit forces it open.
    const pane = paneElement("claude.model-tuning");
    expect(categoryVisible(pane)).toBe(true);
    expect(pane.querySelector("details")?.open).toBe(true);
    expect(pane.querySelector('[data-pref-anchor="reasoning_enabled"]')).toBeTruthy();
  });

  it("search finds fields through synonyms", async () => {
    const user = userEvent.setup();
    render(<Preferences navigate={vi.fn()} />);
    await user.type(screen.getByLabelText("Search settings"), "diarization");
    const results = await screen.findByTestId("pref-search-results");
    await user.click(
      within(results).getByText("Default speakers").closest("button")!
    );
    expect(categoryVisible(paneElement("transcription.imported-audio"))).toBe(
      true
    );
  });

  it("opt-in saved-text search surfaces prompt content", async () => {
    const user = userEvent.setup();
    render(<Preferences navigate={vi.fn()} />);
    await user.click(
      screen.getByLabelText(/Also search your prompts/, { selector: "input" })
    );
    await user.type(screen.getByLabelText("Search settings"), "psychologist");
    const hit = await screen.findByText("Chat System Prompt text");
    await user.click(hit.closest("button")!);
    expect(categoryVisible(paneElement("prompts.chat-system"))).toBe(true);
    expect(
      paneElement("prompts.chat-system").querySelector("details")?.open
    ).toBe(true);
  });

  it("a Writing-tab jump opens the writer templates pane", async () => {
    render(<Preferences navigate={vi.fn()} focusSection="writer-templates" />);
    const manager = await screen.findByTestId("writer-template-manager");
    expect((manager as HTMLDetailsElement).open).toBe(true);
    expect(categoryVisible(manager)).toBe(true);
  });

  it("exports the preferences file from the sidebar", async () => {
    const user = userEvent.setup();
    render(<Preferences navigate={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Export…" }));
    await screen.findByText("Preferences exported.");
  });

  it("imports a preferences file after confirmation", async () => {
    const user = userEvent.setup();
    vi.stubGlobal("confirm", vi.fn().mockReturnValue(true));
    try {
      render(<Preferences navigate={vi.fn()} />);
      await user.click(screen.getByRole("button", { name: "Import…" }));
      await screen.findByText("Preferences imported.");
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("batches transcription edits into one save when leaving the section", async () => {
    const user = userEvent.setup();
    render(<Preferences navigate={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Transcription" }));
    const spanish = await screen.findByLabelText(/All Spanish audio/);
    const saveMock = vi.mocked(savePreferencesPatch);
    saveMock.mockClear();

    await user.click(spanish);
    await user.click(screen.getByLabelText(/Use Transcribe Medical/));
    expect(saveMock).not.toHaveBeenCalled();

    // Leaving the section — a click on the sidebar — flushes one patch with
    // both edits.
    await user.click(screen.getByRole("button", { name: "Claude" }));
    expect(saveMock).toHaveBeenCalledTimes(1);
    expect(saveMock).toHaveBeenCalledWith({
      transcription: {
        default_language: "spanish",
        default_speaker_count: 2,
        use_medical_for_english: true,
        translate_to_english: false,
      },
    });
  });

  it("flushes pending transcription edits when Preferences unmounts", async () => {
    const user = userEvent.setup();
    const { unmount } = render(<Preferences navigate={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Transcription" }));
    const spanish = await screen.findByLabelText(/All Spanish audio/);
    const saveMock = vi.mocked(savePreferencesPatch);
    saveMock.mockClear();

    await user.click(spanish);
    expect(saveMock).not.toHaveBeenCalled();
    unmount();
    expect(saveMock).toHaveBeenCalledTimes(1);
  });

  it("saves the chat streaming choice when leaving the section", async () => {
    const user = userEvent.setup();
    render(<Preferences navigate={vi.fn()} />);
    const wordByWord = await screen.findByLabelText(/Word by word/);
    const saveMock = vi.mocked(savePreferencesPatch);
    saveMock.mockClear();

    await user.click(wordByWord);
    expect(saveMock).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Prompts" }));
    expect(saveMock).toHaveBeenCalledTimes(1);
    expect(saveMock).toHaveBeenCalledWith({ chat_streaming: "token" });
  });

  it("saves the plan gate and the two role models as one draft_pipeline patch", async () => {
    const user = userEvent.setup();
    renderWithModels();
    await user.click(screen.getByRole("button", { name: "Document Writer" }));
    const saveMock = vi.mocked(savePreferencesPatch);
    saveMock.mockClear();

    await user.click(
      await screen.findByLabelText(/Start drafting as soon as the plan is ready/)
    );
    await user.selectOptions(
      screen.getByLabelText("Planning model"),
      "us.haiku"
    );
    expect(saveMock).not.toHaveBeenCalled();

    // Leaving the section flushes one patch, carrying nothing but the fields
    // this section owns.
    await user.click(screen.getByRole("button", { name: "Claude" }));
    expect(saveMock).toHaveBeenCalledTimes(1);
    expect(saveMock).toHaveBeenCalledWith({
      draft_pipeline: {
        plan_gate: "auto_start",
        planner_model_id: "us.haiku",
        reviewer_model_id: null,
      },
    });
  });

  it("offers each role model an automatic default", async () => {
    const user = userEvent.setup();
    renderWithModels();
    await user.click(screen.getByRole("button", { name: "Document Writer" }));

    const reviewer = (await screen.findByLabelText(
      "Review model"
    )) as HTMLSelectElement;
    expect(reviewer.value).toBe("");
    expect(
      within(reviewer).getByRole("option", {
        name: "Default — chosen automatically",
      })
    ).toBeDefined();
  });

  it("finds the draft run pane through the search index", async () => {
    const user = userEvent.setup();
    renderWithModels();
    await user.type(screen.getByLabelText("Search settings"), "planner");
    const results = await screen.findByTestId("pref-search-results");
    await user.click(
      within(results).getByText("Planning model").closest("button")!
    );
    const pane = paneElement("writer.draft-runs");
    expect(categoryVisible(pane)).toBe(true);
    expect(pane.querySelector("details")?.open).toBe(true);
  });

  it("opens the preferences file version history", async () => {
    const user = userEvent.setup();
    render(<Preferences navigate={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "History" }));
    await screen.findByText("No version history found.");
  });
});
