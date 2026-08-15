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
import { PREFERENCES_NAV } from "../lib/preferencesNav";

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
});
