import {
  getFileVersionText,
  getPreferencesVersion,
  getPromptVersion,
  listFileVersions,
  listPreferencesVersions,
  listPromptVersions,
  restoreFileVersion,
  restorePreferencesVersion,
  restorePromptVersion,
  type FileVersion,
} from "./tauri";
import { versionKeyFor } from "./recordFiles";

/**
 * Where a version list comes from and what can be done with it.
 *
 * Record files and custom prompts are versioned by the same S3 mechanism and
 * exposed through parallel command pairs, so the history UI takes these three
 * operations rather than the identifiers they happen to need.
 */
export type VersionSource = {
  list: () => Promise<FileVersion[]>;
  getText: (versionId: string) => Promise<string>;
  restore: (versionId: string) => Promise<void>;
};

/**
 * History of a record file. Audio files resolve to their transcript sidecar —
 * see `versionKeyFor`, the raw upload has no interesting history.
 */
export function recordFileVersions(
  clientId: string,
  filename: string,
): VersionSource {
  const key = versionKeyFor(filename);
  return {
    list: () => listFileVersions(clientId, key),
    getText: (versionId) => getFileVersionText(clientId, key, versionId),
    restore: (versionId) => restoreFileVersion(clientId, key, versionId),
  };
}

/** History of a custom prompt. */
export function promptVersions(promptName: string): VersionSource {
  return {
    list: () => listPromptVersions(promptName),
    getText: (versionId) => getPromptVersion(promptName, versionId),
    restore: (versionId) => restorePromptVersion(promptName, versionId),
  };
}

/** History of the synced preferences file (`_state/preferences.json`). */
export function preferencesVersions(): VersionSource {
  return {
    list: () => listPreferencesVersions(),
    getText: (versionId) => getPreferencesVersion(versionId),
    restore: (versionId) => restorePreferencesVersion(versionId),
  };
}
