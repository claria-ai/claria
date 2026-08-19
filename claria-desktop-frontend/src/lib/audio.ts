// Pure audio-buffer plumbing for the memo recorder.
//
// The recorder captures mono float PCM at whatever rate the machine's
// AudioContext runs at (typically 44.1 or 48 kHz); transcribe.cpp wants 16 kHz, and
// the Tauri command wants base64. Everything in this module is a plain
// transform over buffers — the stateful capture engine lives in
// `useMemoRecorder`.

/** Concatenate the captured PCM chunks into one contiguous buffer. */
export function mergePcmChunks(chunks: Float32Array[]): Float32Array {
  const totalLen = chunks.reduce((sum, c) => sum + c.length, 0);
  const merged = new Float32Array(totalLen);
  let offset = 0;
  for (const chunk of chunks) {
    merged.set(chunk, offset);
    offset += chunk.length;
  }
  return merged;
}

/**
 * Resample mono float PCM to the 16 kHz transcribe.cpp expects.
 *
 * Rendering through an `OfflineAudioContext` hands the sample-rate conversion
 * to the browser's own resampler rather than hand-rolling interpolation. A
 * buffer already at 16 kHz is returned untouched.
 */
export async function resampleTo16kHz(
  buffer: Float32Array,
  sourceSampleRate: number,
): Promise<Float32Array> {
  if (sourceSampleRate === 16000) return buffer;
  const duration = buffer.length / sourceSampleRate;
  const offlineCtx = new OfflineAudioContext(
    1,
    Math.ceil(duration * 16000),
    16000,
  );
  const audioBuffer = offlineCtx.createBuffer(1, buffer.length, sourceSampleRate);
  audioBuffer.getChannelData(0).set(buffer);
  const source = offlineCtx.createBufferSource();
  source.buffer = audioBuffer;
  source.connect(offlineCtx.destination);
  source.start();
  const rendered = await offlineCtx.startRendering();
  return rendered.getChannelData(0);
}

/** Base64-encode the raw little-endian float32 bytes for the IPC hop. */
export function float32ToBase64(samples: Float32Array): string {
  const bytes = new Uint8Array(
    samples.buffer,
    samples.byteOffset,
    samples.byteLength,
  );
  let binary = "";
  for (let i = 0; i < bytes.length; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

/** `m:ss` clock for the recording bar. */
export function formatElapsed(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}
