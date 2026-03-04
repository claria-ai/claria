// Re-export from generated bindings — the source of truth is the Rust backend.
// tauri-specta generates bindings.ts from #[specta::specta] annotated commands.
// If a command is renamed/removed in Rust, this file will fail to compile.

import { commands } from "./bindings";
export { commands };
export type {
  AccessKeyInfo,
  Action,
  AssumeRoleResult,
  BootstrapResult,
  BootstrapStep,
  CallerIdentity,
  Cause,
  ChatHistoryDetail,
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
  Lifecycle,
  NewCredentials,
  PlanEntry,
  RecordContext,
  RecordFile,
  ResourceSpec,
  Severity,
  StepStatus,
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
// IAM policy escalation
// ---------------------------------------------------------------------------

export async function escalateIamPolicy(
  accessKeyId: string,
  secretAccessKey: string
): Promise<void> {
  unwrap(await commands.escalateIamPolicy(accessKeyId, secretAccessKey));
}

// ---------------------------------------------------------------------------
// Provisioner wrappers
// ---------------------------------------------------------------------------

export async function plan() {
  return unwrap(await commands.plan());
}

export async function apply() {
  return unwrap(await commands.apply());
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

export async function listRecordFiles(clientId: string): Promise<import("./bindings").RecordFile[]> {
  return unwrap(await commands.listRecordFiles(clientId));
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
): Promise<string> {
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
