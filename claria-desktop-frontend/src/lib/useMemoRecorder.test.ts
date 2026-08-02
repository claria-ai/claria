import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  createTextRecordFile: vi.fn(),
  getWhisperModels: vi.fn(),
  transcribeMemo: vi.fn(),
}));

vi.mock("./tauri", () => ({
  createTextRecordFile: mocks.createTextRecordFile,
  getWhisperModels: mocks.getWhisperModels,
  transcribeMemo: mocks.transcribeMemo,
}));

import { defaultMemoStamp, useMemoRecorder } from "./useMemoRecorder";

function installAudioHarness() {
  const stop = vi.fn();
  const stream = {
    getTracks: () => [{ stop }],
  } as unknown as MediaStream;
  const source = {
    connect: vi.fn(),
    disconnect: vi.fn(),
  } as unknown as MediaStreamAudioSourceNode;
  const processor = {
    onaudioprocess: null,
    connect: vi.fn(),
    disconnect: vi.fn(),
  } as unknown as ScriptProcessorNode;
  let state: AudioContextState = "running";
  const context = {
    sampleRate: 16000,
    destination: {},
    get state() {
      return state;
    },
    createMediaStreamSource: vi.fn(() => source),
    createScriptProcessor: vi.fn(() => processor),
    suspend: vi.fn(async () => {
      state = "suspended";
    }),
    resume: vi.fn(async () => {
      state = "running";
    }),
    close: vi.fn(async () => {
      state = "closed";
    }),
  } as unknown as AudioContext;
  const getUserMedia = vi.fn(async () => stream);

  vi.stubGlobal("navigator", { mediaDevices: { getUserMedia } });
  vi.stubGlobal(
    "AudioContext",
    vi.fn(function AudioContextMock() {
      return context;
    })
  );

  return { context, getUserMedia, processor, source, stop };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getWhisperModels.mockResolvedValue([
    {
      active: true,
      tier: "large_v3_turbo",
      gpu_accelerated: true,
      dir_name: "whisper-large-v3-turbo",
    },
  ]);
  mocks.transcribeMemo.mockResolvedValue({
    text: "Captured memo",
    language: "en",
  });
  mocks.createTextRecordFile.mockResolvedValue(undefined);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("useMemoRecorder", () => {
  it("captures PCM and transcribes the current recording", async () => {
    const audio = installAudioHarness();
    const hook = renderHook(() =>
      useMemoRecorder({
        clientId: "client-1",
        onError: vi.fn(),
        onSaved: vi.fn(),
      })
    );

    await act(async () => hook.result.current.start());
    const processAudio = audio.processor.onaudioprocess;
    expect(processAudio).not.toBeNull();
    act(() => {
      processAudio?.call(audio.processor, {
        inputBuffer: {
          getChannelData: () => new Float32Array([0.25, -0.25]),
        },
      } as unknown as AudioProcessingEvent);
    });
    await act(async () => hook.result.current.pause());

    expect(mocks.transcribeMemo).toHaveBeenCalledOnce();
    expect(hook.result.current.transcript).toBe("Captured memo");
    expect(hook.result.current.detectedLanguage).toBe("en");
  });

  it("releases the microphone and audio graph when its tab unmounts", async () => {
    const audio = installAudioHarness();
    const hook = renderHook(() =>
      useMemoRecorder({
        clientId: "client-1",
        onError: vi.fn(),
        onSaved: vi.fn(),
      })
    );
    await act(async () => hook.result.current.start());

    hook.unmount();

    expect(audio.stop).toHaveBeenCalledOnce();
    expect(audio.processor.disconnect).toHaveBeenCalledOnce();
    expect(audio.source.disconnect).toHaveBeenCalledOnce();
    expect(audio.context.close).toHaveBeenCalledOnce();
    expect(audio.processor.onaudioprocess).toBeNull();
  });
});

/**
 * The recorder seeds the filename shown after a recording, so it has to sort
 * and pad predictably — `memo-2026-3-1-9-5` would sort incorrectly.
 */
describe("defaultMemoStamp", () => {
  it("pads every field to a fixed width", () => {
    // Local time; month is 0-based in the Date constructor.
    expect(defaultMemoStamp(new Date(2026, 2, 1, 9, 5))).toBe("20260301-0905");
  });

  it("keeps two-digit fields intact", () => {
    expect(defaultMemoStamp(new Date(2026, 10, 25, 14, 30))).toBe(
      "20261125-1430"
    );
  });

  it("renders midnight as 0000, not blank", () => {
    expect(defaultMemoStamp(new Date(2026, 0, 1, 0, 0))).toBe("20260101-0000");
  });

  it("sorts lexicographically in chronological order", () => {
    const stamps = [
      defaultMemoStamp(new Date(2026, 11, 31, 23, 59)),
      defaultMemoStamp(new Date(2026, 0, 2, 0, 0)),
      defaultMemoStamp(new Date(2026, 0, 1, 9, 5)),
    ];
    expect([...stamps].sort()).toEqual([
      "20260101-0905",
      "20260102-0000",
      "20261231-2359",
    ]);
  });
});
