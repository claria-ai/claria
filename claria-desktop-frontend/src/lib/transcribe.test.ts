import { describe, expect, it } from "vitest";
import type { TranscriptionPreferences } from "./tauri";
import {
  transcribeEngineSummary,
  transcribeSummary,
  transcribeTooltip,
  usesMedicalEngine,
} from "./transcribe";

function prefs(
  overrides: Partial<TranscriptionPreferences> = {},
): TranscriptionPreferences {
  return {
    default_language: "english",
    use_medical_for_english: false,
    default_speaker_count: 2,
    translate_to_english: false,
    ...overrides,
  };
}

describe("usesMedicalEngine", () => {
  it("requires both English and the medical opt-in", () => {
    expect(usesMedicalEngine("english", true)).toBe(true);
    expect(usesMedicalEngine("english", false)).toBe(false);
    expect(usesMedicalEngine("spanish", true)).toBe(false);
    expect(usesMedicalEngine("mixed", true)).toBe(false);
  });
});

describe("transcribeEngineSummary", () => {
  it("names Medical only for opted-in English", () => {
    expect(transcribeEngineSummary("english", true)).toBe(
      "Transcribe Medical (en-US, $0.075/min, PHI tagging)",
    );
  });

  it.each([
    ["english" as const, false, "Transcribe Standard (en-US, $0.024/min)"],
    ["spanish" as const, false, "Transcribe Standard (es-US, $0.024/min)"],
    ["mixed" as const, false, "Transcribe Standard (en-US + es-US, auto-detect)"],
    ["spanish" as const, true, "Transcribe Standard (es-US, $0.024/min)"],
    ["mixed" as const, true, "Transcribe Standard (en-US + es-US, auto-detect)"],
  ])("summarises %s (medical=%s)", (language, medical, expected) => {
    expect(transcribeEngineSummary(language, medical)).toBe(expected);
  });
});

describe("transcribeSummary", () => {
  it("describes the default English setup", () => {
    expect(transcribeSummary(prefs())).toBe(
      "Audio uses: Standard · English · 2 speakers.",
    );
  });

  it("switches to Medical when English has it enabled", () => {
    expect(transcribeSummary(prefs({ use_medical_for_english: true }))).toBe(
      "Audio uses: Medical · English · 2 speakers.",
    );
  });

  it("keeps Standard for Spanish even with the medical flag set", () => {
    expect(
      transcribeSummary(
        prefs({ default_language: "spanish", use_medical_for_english: true }),
      ),
    ).toBe("Audio uses: Standard · Spanish · 2 speakers.");
  });

  it("labels mixed audio", () => {
    expect(transcribeSummary(prefs({ default_language: "mixed" }))).toBe(
      "Audio uses: Standard · Mixed (en+es) · 2 speakers.",
    );
  });

  it("singularises one speaker", () => {
    expect(transcribeSummary(prefs({ default_speaker_count: 1 }))).toBe(
      "Audio uses: Standard · English · 1 speaker.",
    );
  });

  it("appends the translation note", () => {
    expect(transcribeSummary(prefs({ translate_to_english: true }))).toBe(
      "Audio uses: Standard · English · 2 speakers, translate.",
    );
  });
});

describe("transcribeTooltip", () => {
  it("falls back to a generic hint with no preferences loaded", () => {
    expect(transcribeTooltip(null)).toBe("Drag files here to upload");
  });

  it("appends the processing-time heuristic to the summary", () => {
    expect(transcribeTooltip(prefs())).toBe(
      "Audio uses: Standard · English · 2 speakers.\n" +
        "Duration estimate: ~1 minute of processing per 6 minutes of audio.",
    );
  });
});
