// Re-export from generated bindings — the source of truth is the Rust backend.
// tauri-specta generates bindings.ts from #[specta::specta] annotated commands.
// If a command is renamed/removed in Rust, this file will fail to compile.

import { commands } from "./bindings";
export { commands };
export { events } from "./bindings";
export type {
  AccessKeyInfo,
  Action,
  AssumeRoleResult,
  BootstrapResult,
  BootstrapStep,
  CallerIdentity,
  Cause,
  CredentialScope,
  ChatHistoryDetail,
  ChatHistoryDetailMessage,
  ChatMessage,
  ChatModel,
  ChatResponse,
  ChatRole,
  ClientSummary,
  ConfigInfo,
  CredentialAssessment,
  CredentialClass,
  CredentialSource,
  DeletedClient,
  DeletedFile,
  FieldDrift,
  FileVersion,
  InfraChatResponse,
  Lifecycle,
  ModelPricing,
  NewCredentials,
  PlanEntry,
  RecordContext,
  RecordFile,
  ResourceSpec,
  Severity,
  SpeakerMode,
  StepStatus,
  TranscribeOptionsOverrides,
  TranscriptionLanguage,
  TranscriptionPreferences,
  TurnUsage,
  LockState,
  LockStateChanged,
  BiometryAvailability,
} from "./bindings";
export type { Result } from "./bindings";

/**
 * Unwrap a tauri-specta `Result<T, E>` into a plain value or throw.
 *
 * The generated bindings return `{ status: "ok", data: T } | { status: "error", error: E }`
 * instead of throwing. This helper converts that back to a throw-on-error style
 * so existing frontend code doesn't need to change its error handling pattern.
 *
 * Usage:
 *   const config = unwrap(await commands.loadConfig());
 */
export function unwrap<T, E>(result: { status: "ok"; data: T } | { status: "error"; error: E }): T {
  if (result.status === "ok") {
    return result.data;
  }
  throw result.error;
}

// ---------------------------------------------------------------------------
// Convenience async wrappers that call commands and unwrap in one step.
// These preserve the old API shape so existing pages don't need rewriting.
// ---------------------------------------------------------------------------

export async function hasConfig(): Promise<boolean> {
  return unwrap(await commands.hasConfig());
}

export async function loadConfig() {
  return unwrap(await commands.loadConfig());
}

/**
 * Save the synced subset of preferences to both the local config file and
 * `_state/preferences.json` in S3. Throws on S3 write failure with the
 * "saved locally but cloud sync failed" prefix so callers can show a
 * partial-save state.
 */
export async function savePreferences(
  preferredModelId: string | null,
  costExplorerEnabled: boolean,
  hourlyCostData: boolean,
  promptCachingEnabled: boolean,
  transcription: import("./bindings").TranscriptionPreferences
) {
  return unwrap(
    await commands.savePreferences(
      preferredModelId,
      costExplorerEnabled,
      hourlyCostData,
      promptCachingEnabled,
      transcription
    )
  );
}

/**
 * Re-fetch synced preferences from S3 and overlay onto the in-memory config.
 * Used by the Preferences page on entry so the editing machine sees the
 * latest cloud state without an app restart.
 */
export async function fetchCloudPreferences() {
  return unwrap(await commands.fetchCloudPreferences());
}

/**
 * Upload an audio file via the wizard path with per-file option overrides.
 * `overrides=null` means "use the user's saved preferences as-is".
 */
export async function uploadRecordFileWithOptions(
  clientId: string,
  filePath: string,
  overrides: import("./bindings").TranscribeOptionsOverrides | null
) {
  return unwrap(
    await commands.uploadRecordFileWithOptions(clientId, filePath, overrides)
  );
}

/** Persist user edits to a transcript's `.text` sidecar (S3 versioning preserves v1). */
export async function saveTranscriptEdits(
  clientId: string,
  filename: string,
  body: string
): Promise<void> {
  unwrap(await commands.saveTranscriptEdits(clientId, filename, body));
}

/**
 * Open a native file picker scoped to supported audio formats. Returns the
 * absolute path, or `null` if the user cancelled.
 */
export async function pickAudioFile(): Promise<string | null> {
  return unwrap(await commands.pickAudioFile());
}

export async function saveConfig(
  region: string,
  systemName: string,
  accountId: string,
  credentials: import("./bindings").CredentialSource
): Promise<void> {
  const result = await commands.saveConfig(region, systemName, accountId, credentials);
  unwrap(result);
}

export async function deleteConfig(): Promise<void> {
  const result = await commands.deleteConfig();
  unwrap(result);
}

export async function assessCredentials(
  region: string,
  credentials: import("./bindings").CredentialSource
) {
  return unwrap(await commands.assessCredentials(region, credentials));
}

/**
 * Assume a role in an AWS sub-account using parent-account credentials.
 *
 * Returns temporary credentials (with session token) that can be fed into
 * `assessCredentials` and `bootstrapIamUser` to set up a dedicated IAM user
 * in the sub-account.
 */
export async function assumeRole(
  region: string,
  credentials: import("./bindings").CredentialSource,
  accountId: string,
  roleName: string
) {
  return unwrap(
    await commands.assumeRole(region, credentials, accountId, roleName)
  );
}

export async function bootstrapIamUser(
  region: string,
  systemName: string,
  rootAccessKeyId: string,
  rootSecretAccessKey: string,
  sessionToken: string | null,
  credentialClass: import("./bindings").CredentialClass
) {
  return unwrap(
    await commands.bootstrapIamUser(
      region,
      systemName,
      rootAccessKeyId,
      rootSecretAccessKey,
      sessionToken,
      credentialClass
    )
  );
}

export async function listAwsProfiles(): Promise<string[]> {
  return unwrap(await commands.listAwsProfiles());
}

export async function listUserAccessKeys(
  region: string,
  credentials: import("./bindings").CredentialSource
) {
  return unwrap(
    await commands.listUserAccessKeys(region, credentials)
  );
}

export async function deleteUserAccessKey(
  region: string,
  credentials: import("./bindings").CredentialSource,
  accessKeyId: string
): Promise<void> {
  unwrap(
    await commands.deleteUserAccessKey(region, credentials, accessKeyId)
  );
}

// ---------------------------------------------------------------------------
// Provisioner progress types
// ---------------------------------------------------------------------------

export type ProvisionerProgress =
  | { kind: "scan_started"; label: string; index: number; total: number }
  | { kind: "scan_completed"; label: string; index: number; total: number }
  | { kind: "apply_started"; label: string; action: string; index: number; total: number }
  | { kind: "apply_completed"; label: string; action: string; index: number; total: number }
  | { kind: "escalation_step"; label: string; status: string };

// ---------------------------------------------------------------------------
// Unified provision wrappers
// ---------------------------------------------------------------------------

export interface ProvisionScanResult {
  entries: import("./bindings").PlanEntry[];
  needs_escalation: boolean;
  account_id: string;
}

export async function provisionScan(
  region: string,
  systemName: string,
  credentials: import("./bindings").CredentialSource,
  onProgress?: (p: ProvisionerProgress) => void
): Promise<ProvisionScanResult> {
  const { invoke, Channel } = await import("@tauri-apps/api/core");
  const channel = new Channel<ProvisionerProgress>();
  if (onProgress) {
    channel.onmessage = onProgress;
  }
  return await invoke("provision_scan", {
    region,
    systemName,
    credentials,
    onProgress: channel,
  });
}

export async function provisionApply(
  region: string,
  systemName: string,
  credentials: import("./bindings").CredentialSource,
  elevatedCredentials: import("./bindings").CredentialSource | null,
  onProgress?: (p: ProvisionerProgress) => void
): Promise<import("./bindings").PlanEntry[]> {
  const { invoke, Channel } = await import("@tauri-apps/api/core");
  const channel = new Channel<ProvisionerProgress>();
  if (onProgress) {
    channel.onmessage = onProgress;
  }
  return await invoke("provision_apply", {
    region,
    systemName,
    credentials,
    elevatedCredentials,
    onProgress: channel,
  });
}

// ---------------------------------------------------------------------------
// Provisioner wrappers — day-2 plan/apply against the saved config
// (used by Provision; InfraChat calls plan() headlessly on mount)
// ---------------------------------------------------------------------------

export async function plan(
  onProgress?: (p: ProvisionerProgress) => void
) {
  const { invoke, Channel } = await import("@tauri-apps/api/core");
  const channel = new Channel<ProvisionerProgress>();
  if (onProgress) {
    channel.onmessage = onProgress;
  }
  return await invoke<import("./bindings").PlanEntry[]>("plan", { onProgress: channel });
}

export async function apply(
  onProgress?: (p: ProvisionerProgress) => void
) {
  const { invoke, Channel } = await import("@tauri-apps/api/core");
  const channel = new Channel<ProvisionerProgress>();
  if (onProgress) {
    channel.onmessage = onProgress;
  }
  return await invoke<import("./bindings").PlanEntry[]>("apply", { onProgress: channel });
}

export async function destroy(): Promise<void> {
  unwrap(await commands.destroy());
}

export async function resetProvisionerState(): Promise<void> {
  unwrap(await commands.resetProvisionerState());
}

// ---------------------------------------------------------------------------
// Client wrappers
// ---------------------------------------------------------------------------

export async function listClients() {
  return unwrap(await commands.listClients());
}

export async function createClient(name: string) {
  return unwrap(await commands.createClient(name));
}

export async function deleteClient(clientId: string): Promise<void> {
  unwrap(await commands.deleteClient(clientId));
}

// ---------------------------------------------------------------------------
// Record file wrappers
// ---------------------------------------------------------------------------

export async function listRecordFiles(clientId: string, prefix?: string): Promise<import("./bindings").RecordFile[]> {
  return unwrap(await commands.listRecordFiles(clientId, prefix ?? null));
}

export async function searchRecordContents(clientId: string, query: string): Promise<string[]> {
  return unwrap(await commands.searchRecordContents(clientId, query));
}

export async function uploadRecordFile(clientId: string, filePath: string): Promise<import("./bindings").RecordFile> {
  return unwrap(await commands.uploadRecordFile(clientId, filePath));
}

export async function deleteRecordFile(clientId: string, filename: string): Promise<void> {
  unwrap(await commands.deleteRecordFile(clientId, filename));
}

export async function getRecordFileText(clientId: string, filename: string): Promise<string> {
  return unwrap(await commands.getRecordFileText(clientId, filename));
}

export async function createTextRecordFile(clientId: string, filename: string, content: string): Promise<import("./bindings").RecordFile> {
  return unwrap(await commands.createTextRecordFile(clientId, filename, content));
}

export async function updateTextRecordFile(clientId: string, filename: string, content: string): Promise<void> {
  unwrap(await commands.updateTextRecordFile(clientId, filename, content));
}

export async function listRecordContext(clientId: string): Promise<import("./bindings").RecordContext[]> {
  return unwrap(await commands.listRecordContext(clientId));
}

export async function extractRecordFile(clientId: string, filename: string): Promise<import("./bindings").RecordContext> {
  return unwrap(await commands.extractRecordFile(clientId, filename));
}

// ---------------------------------------------------------------------------
// Chat wrappers
// ---------------------------------------------------------------------------

export async function listChatModels() {
  return unwrap(await commands.listChatModels());
}

export async function chatMessage(clientId: string, modelId: string, messages: import("./bindings").ChatMessage[], chatId?: string | null, contextFilenames?: string[]) {
  return unwrap(await commands.chatMessage(clientId, modelId, messages, chatId ?? null, contextFilenames ?? []));
}

export async function infraChat(
  modelId: string,
  messages: import("./bindings").ChatMessage[],
  planEntries: import("./bindings").PlanEntry[]
): Promise<import("./bindings").InfraChatResponse> {
  return unwrap(await commands.infraChat(modelId, messages, planEntries));
}

export async function acceptModelAgreement(modelId: string): Promise<void> {
  unwrap(await commands.acceptModelAgreement(modelId));
}

export async function loadChatHistory(clientId: string, chatId: string): Promise<import("./bindings").ChatHistoryDetail> {
  return unwrap(await commands.loadChatHistory(clientId, chatId));
}

// ---------------------------------------------------------------------------
// Preferences wrappers
// ---------------------------------------------------------------------------

export async function setPreferredModel(modelId: string | null): Promise<void> {
  unwrap(await commands.setPreferredModel(modelId));
}

// ---------------------------------------------------------------------------
// Prompt wrappers — generic CRUD for named prompts under claria-prompts/
// ---------------------------------------------------------------------------

export async function getPrompt(promptName: string): Promise<string> {
  return unwrap(await commands.getPrompt(promptName));
}

export async function savePrompt(promptName: string, content: string): Promise<void> {
  unwrap(await commands.savePrompt(promptName, content));
}

export async function deletePrompt(promptName: string): Promise<void> {
  unwrap(await commands.deletePrompt(promptName));
}

export async function listPromptVersions(promptName: string): Promise<import("./bindings").FileVersion[]> {
  return unwrap(await commands.listPromptVersions(promptName));
}

export async function getPromptVersion(promptName: string, versionId: string): Promise<string> {
  return unwrap(await commands.getPromptVersion(promptName, versionId));
}

export async function restorePromptVersion(promptName: string, versionId: string): Promise<void> {
  unwrap(await commands.restorePromptVersion(promptName, versionId));
}

// ---------------------------------------------------------------------------
// Version history wrappers
// ---------------------------------------------------------------------------

export async function listFileVersions(clientId: string, filename: string): Promise<import("./bindings").FileVersion[]> {
  return unwrap(await commands.listFileVersions(clientId, filename));
}

export async function getFileVersionText(clientId: string, filename: string, versionId: string): Promise<string> {
  return unwrap(await commands.getFileVersionText(clientId, filename, versionId));
}

export async function restoreFileVersion(clientId: string, filename: string, versionId: string): Promise<void> {
  unwrap(await commands.restoreFileVersion(clientId, filename, versionId));
}

export async function listDeletedFiles(clientId: string): Promise<import("./bindings").DeletedFile[]> {
  return unwrap(await commands.listDeletedFiles(clientId));
}

export async function restoreDeletedFile(clientId: string, filename: string, versionId: string): Promise<void> {
  unwrap(await commands.restoreDeletedFile(clientId, filename, versionId));
}

export async function listDeletedClients(): Promise<import("./bindings").DeletedClient[]> {
  return unwrap(await commands.listDeletedClients());
}

export async function restoreClient(clientId: string, versionId: string): Promise<void> {
  unwrap(await commands.restoreClient(clientId, versionId));
}

// ---------------------------------------------------------------------------
// Whisper model management + local transcription
// ---------------------------------------------------------------------------

export type { WhisperModelInfo, WhisperModelTier, TranscribeMemoResult, UpdateCheck } from "./bindings";

export async function getWhisperModels(): Promise<import("./bindings").WhisperModelInfo[]> {
  return unwrap(await commands.getWhisperModels());
}

export async function downloadWhisperModel(tier: import("./bindings").WhisperModelTier): Promise<import("./bindings").WhisperModelInfo[]> {
  return unwrap(await commands.downloadWhisperModel(tier));
}

export async function deleteWhisperModel(tier: import("./bindings").WhisperModelTier): Promise<import("./bindings").WhisperModelInfo[]> {
  return unwrap(await commands.deleteWhisperModel(tier));
}

export async function deleteWhisperModelDir(dirName: string): Promise<import("./bindings").WhisperModelInfo[]> {
  return unwrap(await commands.deleteWhisperModelDir(dirName));
}

export async function setActiveWhisperModel(tier: import("./bindings").WhisperModelTier): Promise<import("./bindings").WhisperModelInfo[]> {
  return unwrap(await commands.setActiveWhisperModel(tier));
}

export async function transcribeMemo(audioPcmBase64: string): Promise<import("./bindings").TranscribeMemoResult> {
  return unwrap(await commands.transcribeMemo(audioPcmBase64));
}

// ---------------------------------------------------------------------------
// Update check
// ---------------------------------------------------------------------------

export async function checkForUpdates(): Promise<import("./bindings").UpdateCheck> {
  return unwrap(await commands.checkForUpdates());
}

// ---------------------------------------------------------------------------
// Cost Explorer
// ---------------------------------------------------------------------------

export type { CostGranularity, CostAndUsageResult, CostTimePeriod, CostResultGroup } from "./bindings";

export async function getCostAndUsage(
  startDate: string,
  endDate: string,
  granularity: import("./bindings").CostGranularity,
  groupByService: boolean
): Promise<import("./bindings").CostAndUsageResult> {
  return unwrap(await commands.getCostAndUsage(startDate, endDate, granularity, groupByService));
}

export async function probeCostExplorer(): Promise<void> {
  unwrap(await commands.probeCostExplorer());
}

export async function enableCostExplorer(): Promise<void> {
  unwrap(await commands.enableCostExplorer());
}

export async function setHourlyCostData(enabled: boolean): Promise<void> {
  unwrap(await commands.setHourlyCostData(enabled));
}

// ---------------------------------------------------------------------------
// Pricing lookup
// ---------------------------------------------------------------------------

/**
 * Look up Bedrock pricing for a model_id (inference profile or bare
 * foundation id). Returns `null` for unknown models so callers can hide
 * the pre-flight estimate rather than render `$NaN`.
 */
export async function lookupModelPricing(
  modelId: string
): Promise<import("./bindings").ModelPricing | null> {
  return unwrap(await commands.lookupModelPricing(modelId));
}

// ---------------------------------------------------------------------------
// Shell / URL helpers
// ---------------------------------------------------------------------------

export async function openUrl(url: string): Promise<void> {
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("open_url", { url });
}

// ---------------------------------------------------------------------------
// Token counting
// ---------------------------------------------------------------------------

export async function countClientContextTokens(clientId: string, modelId: string, contextFilenames: string[]): Promise<number> {
  return unwrap(await commands.countClientContextTokens(clientId, modelId, contextFilenames));
}

export async function countInfraContextTokens(modelId: string, planEntries: import("./bindings").PlanEntry[]): Promise<number> {
  return unwrap(await commands.countInfraContextTokens(modelId, planEntries));
}

// ---------------------------------------------------------------------------
// Session lock (auto-lock / PIN / biometric unlock)
// ---------------------------------------------------------------------------

export async function getLockState() {
  return unwrap(await commands.getLockState());
}

export async function recordActivity(): Promise<void> {
  unwrap(await commands.recordActivity());
}

export async function lockSession(): Promise<void> {
  unwrap(await commands.lockSession());
}

export async function unlockWithPin(pin: string): Promise<void> {
  unwrap(await commands.unlockWithPin(pin));
}

export async function unlockWithBiometric(): Promise<void> {
  unwrap(await commands.unlockWithBiometric());
}

export async function getBiometryStatus() {
  return unwrap(await commands.getBiometryStatus());
}

export async function enableAutoLock(pin: string, timeoutMinutes: number): Promise<void> {
  unwrap(await commands.enableAutoLock(pin, timeoutMinutes));
}

export async function disableAutoLock(pin: string): Promise<void> {
  unwrap(await commands.disableAutoLock(pin));
}

export async function changePin(currentPin: string, newPin: string): Promise<void> {
  unwrap(await commands.changePin(currentPin, newPin));
}

export async function setAutoLockTimeout(timeoutMinutes: number): Promise<void> {
  unwrap(await commands.setAutoLockTimeout(timeoutMinutes));
}

export async function setBiometricUnlock(enabled: boolean): Promise<void> {
  unwrap(await commands.setBiometricUnlock(enabled));
}

// ---------------------------------------------------------------------------
// Console
// ---------------------------------------------------------------------------

export interface ConsoleEntry {
  timestamp: string;
  level: string;
  target: string;
  message: string;
}

export async function getConsoleLogs(): Promise<ConsoleEntry[]> {
  const { invoke } = await import("@tauri-apps/api/core");
  return await invoke("get_console_logs");
}

export async function getConsoleLogsText(): Promise<string> {
  const { invoke } = await import("@tauri-apps/api/core");
  return await invoke("get_console_logs_text");
}

export async function saveConsoleLogs(): Promise<boolean> {
  const { invoke } = await import("@tauri-apps/api/core");
  return await invoke("save_console_logs");
}
