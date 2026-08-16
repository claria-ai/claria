// Re-export from generated bindings — the source of truth is the Rust backend.
// tauri-specta generates bindings.ts from #[specta::specta] annotated commands.
// If a command is renamed/removed in Rust, this file will fail to compile.
//
// Every wrapper below goes through `commands.*`; nothing in the frontend calls
// `invoke()` with a hand-written command name or a hand-asserted return type,
// because that would move the failure from `tsc` to runtime. Channel-carrying
// commands are no exception — specta types those too, as
// `TAURI_CHANNEL<ProvisionerProgress>`.

import { Channel } from "@tauri-apps/api/core";
import { commands } from "./bindings";
import type {
  ChatHistoryDetail,
  ChatHistorySummary,
  ChatMessage,
  ChatStreamEvent,
  ConfigInfo,
  ConsoleDelta,
  CostAndUsageResult,
  CostGranularity,
  CredentialClass,
  CredentialInput,
  CredentialSource,
  DeletedClient,
  DeletedFile,
  EditorHistoryEntry,
  FileVersion,
  FindingAction,
  FullReportGenerationResponse,
  InfraChatResponse,
  LocalModelId,
  LocalTranscriptionSettings,
  LocalTranscriptionStatus,
  ModelDownloadProgress,
  ModelPricing,
  PlanEntry,
  PreferencesPatch,
  ProvisionApplyOutcome,
  ProvisionScanResult,
  ProvisionerProgress,
  RecordContext,
  RecordFile,
  ReportBlockReferenceInput,
  ReportDraftEdit,
  ReportDraft,
  ReportExportResult,
  ReportFindings,
  ReportFindingResolution,
  ReportProposalChoice,
  ReportRevisionView,
  ReportTemplatePreview,
  ReportTurnProgressView,
  ReportTurnResponse,
  ReportWorkspaceView,
  TranscribeMemoResult,
  TranscribeOptionsOverrides,
  UpdateCheck,
  WriterPrompt,
  WriterTemplateView,
  WriterTrustRules,
} from "./bindings";

export { commands };
export type {
  AccessKeyInfo,
  AccessKeyLimitReached,
  Action,
  AssumedRoleSession,
  BootstrapOutcome,
  BootstrapStep,
  CallerIdentity,
  Cause,
  CredentialScope,
  ChatHistoryDetail,
  ChatHistoryDetailMessage,
  ChatHistorySummary,
  ChatMessage,
  ChatModel,
  ChatResponse,
  ChatRole,
  ChatStreamEvent,
  ChatStreamMode,
  ClientNameHistoryEntry,
  ClientNameUpdate,
  ClientRecordDetails,
  ClientSummary,
  CacheTtlChoice,
  ConfigInfo,
  ConsoleDelta,
  ConsoleEntry,
  CredentialAssessment,
  CredentialClass,
  CredentialInput,
  CredentialSource,
  DeletedClient,
  DeletedFile,
  EditorHistoryEntry,
  ConflictingRef,
  FieldDrift,
  FileVersion,
  Finding,
  FindingAction,
  FindingAnchor,
  FindingStatus,
  FullReportGenerationResponse,
  InfraChatResponse,
  Lifecycle,
  LocalBackend,
  LocalBackendInfo,
  LocalComputeDevice,
  LocalKvPrecision,
  LocalModelId,
  LocalModelInfo,
  LocalTranscriptionSettings,
  LocalTranscriptionStatus,
  ModelDownloadProgress,
  ModelPricing,
  NewCredentialsInfo,
  PlanEntry,
  PreferencesPatch,
  ProvisionApplyOutcome,
  ProvisionScanResult,
  ProvisionerProgress,
  RecordCitation,
  RecordContext,
  RecordFile,
  ReportAuthoringPreferences,
  ReportAuthoringTurnView,
  ReportBlockReferenceInput,
  ReportBlock,
  ReportContent,
  ReportContextFileView,
  ReportContextReadView,
  ReportDraftEdit,
  ReportDraft,
  ReportExportResult,
  ReportExportStatus,
  ReportExport,
  ReportFindings,
  ReportFindingResolution,
  ReportOperation,
  ReportProposalChoice,
  ReportProposalDecision,
  ReportProposalResolution,
  ReportProposalView,
  ReportRevisionView,
  ReportSectionEdit,
  ReportTemplateImportView,
  ReportTemplatePreview,
  ReportTemplateStatsView,
  ReportTemplateWarningView,
  ReportSection,
  ReportTimelineItemView,
  ReportTimelineRole,
  ReportToolActivityStatus,
  ReportTurnProgressView,
  ReportTurnResponse,
  ReportWorkspaceView,
  ResourceSpec,
  ReviewCoverage,
  ReviewPass,
  StyleProposal,
  TextSpan,
  Severity,
  SpeakerMode,
  StepStatus,
  EffortPreference,
  ModelTuningPreferences,
  TranscribeOptionsOverrides,
  TranscriptionLanguage,
  TranscriptionPreferences,
  TurnUsage,
  WriterTemplateView,
  WriterTrustRules,
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
 * Save one preferences section's fields. Absent patch fields are left
 * untouched locally and in `_state/preferences.json`, so sections can't
 * clobber each other. Throws on S3 write failure with the "saved locally
 * but cloud sync failed" prefix so callers can show a partial-save state.
 */
export async function savePreferencesPatch(patch: PreferencesPatch) {
  return unwrap(await commands.savePreferencesPatch(patch));
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
  overrides: TranscribeOptionsOverrides | null
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
  credentials: CredentialSource
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
  credentials: CredentialInput
) {
  return unwrap(await commands.assessCredentials(region, credentials));
}

/**
 * Assume a role in an AWS sub-account using parent-account credentials.
 *
 * Returns an `AssumedRoleSession`: the session's metadata plus an opaque
 * handle that later provisioning calls pass as
 * `{ type: "assumed_role", handle }` — the temporary secrets stay in the
 * Rust backend and never reach the frontend.
 */
export async function assumeRole(
  region: string,
  credentials: CredentialInput,
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
  credentialClass: CredentialClass
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
  credentials: CredentialInput
) {
  return unwrap(
    await commands.listUserAccessKeys(region, credentials)
  );
}

export async function deleteUserAccessKey(
  region: string,
  credentials: CredentialInput,
  accessKeyId: string
): Promise<void> {
  unwrap(
    await commands.deleteUserAccessKey(region, credentials, accessKeyId)
  );
}

// ---------------------------------------------------------------------------
// Provisioner wrappers
//
// These stream `ProvisionerProgress` over a Tauri channel. The generated
// bindings take the channel as an argument, so callers pass a plain callback
// and the channel plumbing stays here.
// ---------------------------------------------------------------------------

/** Wrap an optional progress callback in the channel the bindings expect. */
function progressChannel(
  onProgress?: (p: ProvisionerProgress) => void
): Channel<ProvisionerProgress> {
  const channel = new Channel<ProvisionerProgress>();
  if (onProgress) {
    channel.onmessage = onProgress;
  }
  return channel;
}

export async function provisionScan(
  region: string,
  systemName: string,
  credentials: CredentialInput,
  onProgress?: (p: ProvisionerProgress) => void
): Promise<ProvisionScanResult> {
  return unwrap(
    await commands.provisionScan(
      region,
      systemName,
      credentials,
      progressChannel(onProgress)
    )
  );
}

export async function provisionApply(
  region: string,
  systemName: string,
  credentials: CredentialInput,
  elevatedCredentials: CredentialInput | null,
  onProgress?: (p: ProvisionerProgress) => void
): Promise<ProvisionApplyOutcome> {
  return unwrap(
    await commands.provisionApply(
      region,
      systemName,
      credentials,
      elevatedCredentials,
      progressChannel(onProgress)
    )
  );
}

// Day-2 plan/apply against the saved config
// (used by Provision; InfraChat calls plan() headlessly on mount)

export async function plan(
  onProgress?: (p: ProvisionerProgress) => void
): Promise<PlanEntry[]> {
  return unwrap(await commands.plan(progressChannel(onProgress)));
}

export async function apply(
  onProgress?: (p: ProvisionerProgress) => void
): Promise<PlanEntry[]> {
  return unwrap(await commands.apply(progressChannel(onProgress)));
}

export async function destroy(
  elevatedCredentials: CredentialInput
): Promise<void> {
  unwrap(await commands.destroy(elevatedCredentials));
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

export async function getClientRecordDetails(clientId: string) {
  return unwrap(await commands.getClientRecordDetails(clientId));
}

export async function updateClientName(clientId: string, name: string) {
  return unwrap(await commands.updateClientName(clientId, name));
}

export async function deleteClient(clientId: string): Promise<void> {
  unwrap(await commands.deleteClient(clientId));
}

// ---------------------------------------------------------------------------
// Writing workspace wrappers
// ---------------------------------------------------------------------------

export async function startReportWorkspace(
  clientId: string,
  reportId: string
): Promise<ReportWorkspaceView> {
  return unwrap(await commands.startReportWorkspace(clientId, reportId));
}

export async function loadReportWorkspace(
  clientId: string,
  reportId: string
): Promise<ReportWorkspaceView> {
  return unwrap(await commands.loadReportWorkspace(clientId, reportId));
}

export async function listEditorHistory(
  clientId: string
): Promise<EditorHistoryEntry[]> {
  return unwrap(await commands.listEditorHistory(clientId));
}

export async function renameReportSession(
  clientId: string,
  reportId: string,
  name: string
): Promise<ReportWorkspaceView> {
  return unwrap(await commands.renameReportSession(clientId, reportId, name));
}

export async function listReportRevisions(
  clientId: string,
  reportId: string
): Promise<ReportRevisionView[]> {
  return unwrap(await commands.listReportRevisions(clientId, reportId));
}

export async function loadReportRevision(
  clientId: string,
  reportId: string,
  revision: number
): Promise<ReportDraft> {
  return unwrap(await commands.loadReportRevision(clientId, reportId, revision));
}

export async function revertReportRevision(
  clientId: string,
  reportId: string,
  expectedRevision: number,
  revision: number
): Promise<ReportWorkspaceView> {
  return unwrap(
    await commands.revertReportRevision(
      clientId,
      reportId,
      expectedRevision,
      revision
    )
  );
}

export async function saveReportDraft(
  clientId: string,
  reportId: string,
  expectedRevision: number,
  draft: ReportDraftEdit
): Promise<ReportWorkspaceView> {
  return unwrap(
    await commands.saveReportDraft(clientId, reportId, expectedRevision, draft)
  );
}

export async function discardQueuedReportEdits(
  clientId: string,
  reportId: string,
  expectedRevision: number
): Promise<ReportWorkspaceView> {
  return unwrap(
    await commands.discardQueuedReportEdits(clientId, reportId, expectedRevision)
  );
}

export async function listWriterTemplates(): Promise<WriterTemplateView[]> {
  return unwrap(await commands.listWriterTemplates());
}

export async function uploadWriterTemplate(): Promise<WriterTemplateView | null> {
  return unwrap(await commands.uploadWriterTemplate());
}

export async function renameWriterTemplate(
  templateId: string,
  name: string
): Promise<WriterTemplateView> {
  return unwrap(await commands.renameWriterTemplate(templateId, name));
}

export async function deleteWriterTemplate(templateId: string): Promise<void> {
  unwrap(await commands.deleteWriterTemplate(templateId));
}

export async function previewWriterTemplate(
  clientId: string,
  templateId: string
): Promise<ReportTemplatePreview> {
  return unwrap(await commands.previewWriterTemplate(clientId, templateId));
}

export async function applyReportTemplate(
  clientId: string,
  reportId: string,
  expectedRevision: number,
  importId: string
): Promise<ReportWorkspaceView> {
  return unwrap(
    await commands.applyReportTemplate(clientId, reportId, expectedRevision, importId)
  );
}

export async function discardReportTemplatePreview(
  importId: string
): Promise<void> {
  unwrap(await commands.discardReportTemplatePreview(importId));
}

export async function generateFullReport(
  clientId: string,
  reportId: string,
  expectedRevision: number,
  modelId: string,
  guidance: string,
  onProgress?: (progress: ReportTurnProgressView) => void
): Promise<FullReportGenerationResponse> {
  const channel = new Channel<ReportTurnProgressView>();
  if (onProgress) channel.onmessage = onProgress;
  return unwrap(
    await commands.generateFullReport(
      clientId,
      reportId,
      expectedRevision,
      modelId,
      guidance,
      channel
    )
  );
}

export async function sendReportMessage(
  clientId: string,
  reportId: string,
  expectedRevision: number,
  modelId: string,
  instruction: string,
  references: ReportBlockReferenceInput[] = [],
  onProgress?: (progress: ReportTurnProgressView) => void
): Promise<ReportTurnResponse> {
  const channel = new Channel<ReportTurnProgressView>();
  if (onProgress) channel.onmessage = onProgress;
  return unwrap(
    await commands.sendReportMessage(
      clientId,
      reportId,
      expectedRevision,
      modelId,
      instruction,
      references,
      channel
    )
  );
}

export async function resolveReportProposal(
  clientId: string,
  reportId: string,
  proposalId: string,
  decision: ReportProposalChoice
): Promise<ReportWorkspaceView> {
  return unwrap(
    await commands.resolveReportProposal(clientId, reportId, proposalId, decision)
  );
}

export async function listReportFindings(
  clientId: string,
  reportId: string
): Promise<ReportFindings> {
  return unwrap(await commands.listReportFindings(clientId, reportId));
}

export async function resolveReportFinding(
  clientId: string,
  reportId: string,
  findingId: string,
  action: FindingAction
): Promise<ReportFindingResolution> {
  return unwrap(
    await commands.resolveReportFinding(clientId, reportId, findingId, action)
  );
}

export async function exportReportDocx(
  clientId: string,
  reportId: string,
  expectedRevision: number
): Promise<ReportExportResult> {
  return unwrap(
    await commands.exportReportDocx(clientId, reportId, expectedRevision)
  );
}

// ---------------------------------------------------------------------------
// Record file wrappers
// ---------------------------------------------------------------------------

export async function listRecordFiles(clientId: string, prefix?: string): Promise<RecordFile[]> {
  return unwrap(await commands.listRecordFiles(clientId, prefix ?? null));
}

export async function searchRecordContents(clientId: string, query: string): Promise<string[]> {
  return unwrap(await commands.searchRecordContents(clientId, query));
}

export async function uploadRecordFile(clientId: string, filePath: string): Promise<RecordFile> {
  return unwrap(await commands.uploadRecordFile(clientId, filePath));
}

export async function deleteRecordFile(clientId: string, filename: string): Promise<void> {
  unwrap(await commands.deleteRecordFile(clientId, filename));
}

export async function getRecordFileText(clientId: string, filename: string): Promise<string> {
  return unwrap(await commands.getRecordFileText(clientId, filename));
}

export async function createTextRecordFile(clientId: string, filename: string, content: string): Promise<RecordFile> {
  return unwrap(await commands.createTextRecordFile(clientId, filename, content));
}

export async function updateTextRecordFile(clientId: string, filename: string, content: string): Promise<void> {
  unwrap(await commands.updateTextRecordFile(clientId, filename, content));
}

export async function listRecordContext(clientId: string): Promise<RecordContext[]> {
  return unwrap(await commands.listRecordContext(clientId));
}

export async function extractRecordFile(clientId: string, filename: string): Promise<RecordContext> {
  return unwrap(await commands.extractRecordFile(clientId, filename));
}

// ---------------------------------------------------------------------------
// Chat wrappers
// ---------------------------------------------------------------------------

export async function listChatModels() {
  return unwrap(await commands.listChatModels());
}

/**
 * Streamed-response channel shared by the chat commands. Wire `onEvent` to
 * receive incremental deltas; callers that omit it (tests, scripts) still
 * get the complete response from the command's return value.
 */
function chatStreamChannel(
  onEvent?: (event: ChatStreamEvent) => void
): Channel<ChatStreamEvent> {
  const channel = new Channel<ChatStreamEvent>();
  if (onEvent) {
    channel.onmessage = onEvent;
  }
  return channel;
}

export async function chatMessage(
  clientId: string,
  modelId: string,
  messages: ChatMessage[],
  streamId: string,
  chatId?: string | null,
  contextFilenames?: string[],
  chatName?: string | null,
  onEvent?: (event: ChatStreamEvent) => void
) {
  return unwrap(
    await commands.chatMessage(
      clientId,
      modelId,
      messages,
      chatId ?? null,
      chatName ?? null,
      contextFilenames ?? [],
      streamId,
      chatStreamChannel(onEvent)
    )
  );
}

export async function infraChat(
  modelId: string,
  messages: ChatMessage[],
  planEntries: PlanEntry[],
  streamId: string,
  onEvent?: (event: ChatStreamEvent) => void
): Promise<InfraChatResponse> {
  return unwrap(
    await commands.infraChat(
      modelId,
      messages,
      planEntries,
      streamId,
      chatStreamChannel(onEvent)
    )
  );
}

/**
 * End the in-flight turn identified by `streamId`. Whatever text already
 * arrived is kept, and the command that is streaming it returns normally
 * with a `stopped_by_user` stop reason. Safe to call for a turn that has
 * already finished.
 */
export async function stopChatStream(streamId: string): Promise<void> {
  unwrap(await commands.stopChatStream(streamId));
}

export async function acceptModelAgreement(modelId: string): Promise<void> {
  unwrap(await commands.acceptModelAgreement(modelId));
}

export async function listChatHistories(
  clientId: string
): Promise<ChatHistorySummary[]> {
  return unwrap(await commands.listChatHistories(clientId));
}

export async function loadChatHistory(clientId: string, chatId: string): Promise<ChatHistoryDetail> {
  return unwrap(await commands.loadChatHistory(clientId, chatId));
}

export async function renameChatHistory(
  clientId: string,
  chatId: string,
  name: string
): Promise<ChatHistoryDetail> {
  return unwrap(await commands.renameChatHistory(clientId, chatId, name));
}

// ---------------------------------------------------------------------------
// Preferences wrappers
// ---------------------------------------------------------------------------

export async function setPreferredModel(modelId: string | null): Promise<void> {
  unwrap(await commands.setPreferredModel(modelId));
}

// ---------------------------------------------------------------------------
// Preferences file wrappers — export, import, and version history for
// _state/preferences.json
// ---------------------------------------------------------------------------

/** Save the synced preferences file locally. Resolves false on cancel. */
export async function exportPreferences(): Promise<boolean> {
  return unwrap(await commands.exportPreferences());
}

/** Replace the synced preferences from a local export. Null on cancel. */
export async function importPreferences(): Promise<ConfigInfo | null> {
  return unwrap(await commands.importPreferences());
}

export async function listPreferencesVersions(): Promise<FileVersion[]> {
  return unwrap(await commands.listPreferencesVersions());
}

export async function getPreferencesVersion(versionId: string): Promise<string> {
  return unwrap(await commands.getPreferencesVersion(versionId));
}

export async function restorePreferencesVersion(versionId: string): Promise<void> {
  unwrap(await commands.restorePreferencesVersion(versionId));
}

// ---------------------------------------------------------------------------
// Prompt wrappers — generic CRUD for named prompts under claria-prompts/
// ---------------------------------------------------------------------------

export async function getPrompt(promptName: string): Promise<string> {
  return unwrap(await commands.getPrompt(promptName));
}

export async function getWriterTrustRules(): Promise<WriterTrustRules> {
  return unwrap(await commands.getWriterTrustRules());
}

export async function savePrompt(promptName: string, content: string): Promise<void> {
  unwrap(await commands.savePrompt(promptName, content));
}

export async function deletePrompt(promptName: string): Promise<void> {
  unwrap(await commands.deletePrompt(promptName));
}

export async function listPromptVersions(promptName: string): Promise<FileVersion[]> {
  return unwrap(await commands.listPromptVersions(promptName));
}

export async function getPromptVersion(promptName: string, versionId: string): Promise<string> {
  return unwrap(await commands.getPromptVersion(promptName, versionId));
}

export async function restorePromptVersion(promptName: string, versionId: string): Promise<void> {
  unwrap(await commands.restorePromptVersion(promptName, versionId));
}

// ---------------------------------------------------------------------------
// Writer prompt library wrappers — reusable steering prompts the user picks
// to prefill a writer instruction
// ---------------------------------------------------------------------------

export type { WriterPrompt } from "./bindings";

export async function listWriterLibraryPrompts(): Promise<WriterPrompt[]> {
  return unwrap(await commands.listWriterLibraryPrompts());
}

export async function saveWriterLibraryPrompt(
  promptId: string | null,
  name: string,
  body: string
): Promise<WriterPrompt> {
  return unwrap(await commands.saveWriterLibraryPrompt(promptId, name, body));
}

export async function deleteWriterLibraryPrompt(promptId: string): Promise<void> {
  unwrap(await commands.deleteWriterLibraryPrompt(promptId));
}

// ---------------------------------------------------------------------------
// Version history wrappers
// ---------------------------------------------------------------------------

export async function listFileVersions(clientId: string, filename: string): Promise<FileVersion[]> {
  return unwrap(await commands.listFileVersions(clientId, filename));
}

export async function getFileVersionText(clientId: string, filename: string, versionId: string): Promise<string> {
  return unwrap(await commands.getFileVersionText(clientId, filename, versionId));
}

export async function restoreFileVersion(clientId: string, filename: string, versionId: string): Promise<void> {
  unwrap(await commands.restoreFileVersion(clientId, filename, versionId));
}

export async function listDeletedFiles(clientId: string): Promise<DeletedFile[]> {
  return unwrap(await commands.listDeletedFiles(clientId));
}

export async function restoreDeletedFile(clientId: string, filename: string): Promise<void> {
  unwrap(await commands.restoreDeletedFile(clientId, filename));
}

export async function listDeletedClients(): Promise<DeletedClient[]> {
  return unwrap(await commands.listDeletedClients());
}

export async function restoreClient(clientId: string): Promise<void> {
  unwrap(await commands.restoreClient(clientId));
}

// ---------------------------------------------------------------------------
// transcribe.cpp model management + local transcription
// ---------------------------------------------------------------------------

export type { TranscribeMemoResult, UpdateCheck } from "./bindings";

export async function getLocalTranscriptionStatus(): Promise<LocalTranscriptionStatus> {
  return unwrap(await commands.getLocalTranscriptionStatus());
}

export async function saveLocalTranscriptionSettings(
  settings: LocalTranscriptionSettings
): Promise<LocalTranscriptionStatus> {
  return unwrap(await commands.saveLocalTranscriptionSettings(settings));
}

export async function downloadLocalModel(
  modelId: LocalModelId,
  onProgress?: (progress: ModelDownloadProgress) => void
): Promise<LocalTranscriptionStatus> {
  const channel = new Channel<ModelDownloadProgress>();
  if (onProgress) channel.onmessage = onProgress;
  return unwrap(await commands.downloadLocalModel(modelId, channel));
}

export async function deleteLocalModel(
  modelId: LocalModelId
): Promise<LocalTranscriptionStatus> {
  return unwrap(await commands.deleteLocalModel(modelId));
}

export async function deleteLegacyTranscriptionModels(): Promise<LocalTranscriptionStatus> {
  return unwrap(await commands.deleteLegacyTranscriptionModels());
}

export async function transcribeMemo(audioPcmBase64: string): Promise<TranscribeMemoResult> {
  return unwrap(await commands.transcribeMemo(audioPcmBase64));
}

// ---------------------------------------------------------------------------
// Update check
// ---------------------------------------------------------------------------

export async function checkForUpdates(): Promise<UpdateCheck> {
  return unwrap(await commands.checkForUpdates());
}

// ---------------------------------------------------------------------------
// Cost Explorer
// ---------------------------------------------------------------------------

export type { CostGranularity, CostAndUsageResult, CostTimePeriod, CostResultGroup } from "./bindings";

export async function getCostAndUsage(
  startDate: string,
  endDate: string,
  granularity: CostGranularity,
  groupByService: boolean
): Promise<CostAndUsageResult> {
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
): Promise<ModelPricing | null> {
  return unwrap(await commands.lookupModelPricing(modelId));
}

// ---------------------------------------------------------------------------
// Shell / URL helpers
// ---------------------------------------------------------------------------

export async function openUrl(url: string): Promise<void> {
  unwrap(await commands.openUrl(url));
}

// ---------------------------------------------------------------------------
// Token counting
// ---------------------------------------------------------------------------

export async function countClientContextTokens(clientId: string, contextFilenames: string[]): Promise<number> {
  return unwrap(await commands.countClientContextTokens(clientId, contextFilenames));
}

export async function countInfraContextTokens(planEntries: PlanEntry[]): Promise<number> {
  return unwrap(await commands.countInfraContextTokens(planEntries));
}

// ---------------------------------------------------------------------------
// Console
// ---------------------------------------------------------------------------

// The console commands are infallible on the Rust side, so the bindings return
// the value directly rather than a `Result` — nothing to unwrap.

export async function getConsoleLogsSince(seq: number): Promise<ConsoleDelta> {
  return await commands.getConsoleLogsSince(seq);
}

export async function getConsoleLogsText(): Promise<string> {
  return await commands.getConsoleLogsText();
}

export async function saveConsoleLogs(): Promise<boolean> {
  return unwrap(await commands.saveConsoleLogs());
}

export async function revealLogFolder(): Promise<void> {
  unwrap(await commands.revealLogFolder());
}
