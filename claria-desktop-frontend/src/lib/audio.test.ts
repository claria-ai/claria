import { afterEach, describe, expect, it } from "vitest";
import {
  float32ToBase64,
  formatElapsed,
  mergePcmChunks,
  resampleTo16kHz,
} from "./audio";

describe("mergePcmChunks", () => {
  it("returns an empty buffer for no chunks", () => {
    expect(mergePcmChunks([]).length).toBe(0);
  });

  it("concatenates chunks in order", () => {
    const merged = mergePcmChunks([
      new Float32Array([1, 2]),
      new Float32Array([3]),
      new Float32Array([4, 5, 6]),
    ]);
    expect(Array.from(merged)).toEqual([1, 2, 3, 4, 5, 6]);
  });

  it("does not alias the source chunks", () => {
    const first = new Float32Array([1, 2]);
    const merged = mergePcmChunks([first, new Float32Array([3])]);
    first[0] = 99;
    expect(merged[0]).toBe(1);
  });

  it("preserves sample values exactly", () => {
    // Float32 round-trips these bit-for-bit; a lossy copy would not.
    const chunk = new Float32Array([-1, -0.5, 0, 0.25, 1]);
    expect(Array.from(mergePcmChunks([chunk]))).toEqual(Array.from(chunk));
  });
});

describe("float32ToBase64", () => {
  it("encodes the raw little-endian float32 bytes", () => {
    // 1.0 is 0x3F800000; little-endian on every platform we ship to.
    const encoded = float32ToBase64(new Float32Array([1]));
    const bytes = Uint8Array.from(atob(encoded), (c) => c.charCodeAt(0));
    expect(Array.from(bytes)).toEqual([0x00, 0x00, 0x80, 0x3f]);
  });

  it("round-trips a buffer through base64", () => {
    const samples = new Float32Array([-1, -0.25, 0, 0.5, 1]);
    const bytes = Uint8Array.from(atob(float32ToBase64(samples)), (c) =>
      c.charCodeAt(0),
    );
    const decoded = new Float32Array(bytes.buffer);
    expect(Array.from(decoded)).toEqual(Array.from(samples));
  });

  it("emits four bytes per sample", () => {
    const samples = new Float32Array(64);
    const bytes = atob(float32ToBase64(samples));
    expect(bytes.length).toBe(256);
  });

  it("encodes an empty buffer as an empty string", () => {
    expect(float32ToBase64(new Float32Array(0))).toBe("");
  });

  it("honours byteOffset on a subarray view", () => {
    // `mergePcmChunks` returns a whole buffer, but a caller passing a view
    // must not silently encode the bytes in front of it.
    const backing = new Float32Array([9, 9, 1]);
    const view = backing.subarray(2);
    const bytes = atob(float32ToBase64(view));
    expect(bytes.length).toBe(4);
  });
});

describe("resampleTo16kHz", () => {
  const original = globalThis.OfflineAudioContext;

  afterEach(() => {
    globalThis.OfflineAudioContext = original;
  });

  it("returns the buffer untouched when already at 16 kHz", async () => {
    const buffer = new Float32Array([0.1, 0.2, 0.3]);
    // Identity, not a copy — the caller hands the result straight to the
    // encoder and an extra allocation per pass would be waste.
    await expect(resampleTo16kHz(buffer, 16000)).resolves.toBe(buffer);
  });

  it("renders through an OfflineAudioContext sized to the 16 kHz duration", async () => {
    const { ctorArgs, factory, rendered } = stubOfflineAudioContext();
    globalThis.OfflineAudioContext = factory;

    // 4410 samples at 44.1 kHz is 0.1s, which is 1600 samples at 16 kHz.
    const out = await resampleTo16kHz(new Float32Array(4410), 44100);

    expect(ctorArgs).toEqual([[1, 1600, 16000]]);
    expect(out).toBe(rendered);
  });

  it("copies the input into the source buffer and starts it", async () => {
    const stub = stubOfflineAudioContext();
    globalThis.OfflineAudioContext = stub.factory;

    const input = new Float32Array([0.5, -0.5, 0.25, -0.25]);
    await resampleTo16kHz(input, 32000);

    expect(Array.from(stub.sourceChannel)).toEqual(Array.from(input));
    expect(stub.started).toBe(true);
    expect(stub.connectedToDestination).toBe(true);
  });

  it("rounds a fractional output length up", async () => {
    const stub = stubOfflineAudioContext();
    globalThis.OfflineAudioContext = stub.factory;

    // 1 sample at 44.1 kHz is 0.36 output samples — must not truncate to 0.
    await resampleTo16kHz(new Float32Array(1), 44100);

    expect(stub.ctorArgs[0][1]).toBe(1);
  });
});

/**
 * A stand-in for the Web Audio pieces `resampleTo16kHz` drives. happy-dom has
 * no audio implementation, so the resampling itself cannot be exercised here —
 * what this pins down is the wiring: the context is sized correctly, the input
 * lands in the source buffer, and the rendered output is what comes back.
 */
function stubOfflineAudioContext() {
  const ctorArgs: number[][] = [];
  let sourceChannel = new Float32Array(0);
  const rendered = new Float32Array([7]);
  const state = { started: false, connectedToDestination: false };

  const destination = {};

  class StubOfflineAudioContext {
    destination = destination;

    constructor(channels: number, length: number, sampleRate: number) {
      ctorArgs.push([channels, length, sampleRate]);
    }

    createBuffer(_channels: number, length: number) {
      sourceChannel = new Float32Array(length);
      return { getChannelData: () => sourceChannel };
    }

    createBufferSource() {
      return {
        buffer: null,
        connect: (node: unknown) => {
          state.connectedToDestination = node === destination;
        },
        start: () => {
          state.started = true;
        },
      };
    }

    startRendering() {
      return Promise.resolve({ getChannelData: () => rendered });
    }
  }

  return {
    ctorArgs,
    rendered,
    factory: StubOfflineAudioContext as unknown as typeof OfflineAudioContext,
    get sourceChannel() {
      return sourceChannel;
    },
    get started() {
      return state.started;
    },
    get connectedToDestination() {
      return state.connectedToDestination;
    },
  };
}

describe("formatElapsed", () => {
  it.each([
    [0, "0:00"],
    [5, "0:05"],
    [59, "0:59"],
    [60, "1:00"],
    [61, "1:01"],
    [600, "10:00"],
    [3599, "59:59"],
    [3600, "60:00"],
  ])("formats %i seconds as %s", (seconds, expected) => {
    expect(formatElapsed(seconds)).toBe(expected);
  });
});
