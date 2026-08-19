// Filename conventions for the objects under `records/{client}/`.
//
// The S3 layout is defined in `claria-core/src/s3_keys.rs`; this module is the
// frontend's read of the two conventions the record page depends on — the
// audio extensions that decide which preview UI a file gets, and the
// `chat-history/` prefix that folds persisted chats into their own folder.

/** Audio containers the backend can transcribe. */
export const AUDIO_EXTENSIONS = new Set([
  "mp3",
  "mp4",
  "m4a",
  "wav",
  "flac",
  "ogg",
  "amr",
  "webm",
]);

/** Prefix of the chat-history objects inside a client's record. */
export const CHAT_HISTORY_PREFIX = "chat-history/";

/** Lowercased extension of a filename, or `""` when it has none. */
export function fileExtension(filename: string): string {
  return filename.split(".").pop()?.toLowerCase() ?? "";
}

/**
 * True when the filename is a supported audio file (e.g. `session.m4a`).
 *
 * The preview modal is opened with the *audio* filename, not the sidecar
 * name — `getRecordFileText` handles the sidecar lookup internally, and
 * `save_transcript_edits` / `restore_original_transcript` also expect the
 * audio filename and append `.text` themselves. So the structured editor
 * gate is on the audio extension, not on a `.text` suffix.
 */
export function isAudioSidecar(filename: string): boolean {
  return AUDIO_EXTENSIONS.has(fileExtension(filename));
}

/**
 * Map an audio filename to the S3 key that the version-history commands
 * should operate on. For audio files this is the `.text` sidecar — the
 * raw audio file's history isn't useful (it's a single immutable
 * upload). For everything else it's the file itself.
 */
export function versionKeyFor(filename: string): string {
  return isAudioSidecar(filename) ? `${filename}.text` : filename;
}

/** True for the persisted-chat objects that get folded into their own folder. */
export function isChatHistory(filename: string): boolean {
  return filename.startsWith(CHAT_HISTORY_PREFIX);
}

/** Recover the chat id from a `chat-history/{uuid}.json` filename. */
export function chatIdFromFilename(filename: string): string {
  return filename.replace(CHAT_HISTORY_PREFIX, "").replace(".json", "");
}

/** The truncated chat id shown in the chat-history folder rows. */
export function chatHistoryLabel(filename: string): string {
  const id = chatIdFromFilename(filename);
  return id.length > 8 ? id.slice(0, 8) + "..." : id;
}

/** Filename from a dropped absolute path, falling back to the whole path. */
export function basename(path: string): string {
  return path.split(/[/\\]/).pop() ?? path;
}
