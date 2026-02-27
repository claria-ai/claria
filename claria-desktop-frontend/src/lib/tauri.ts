// Re-export from generated bindings — the source of truth is the Rust backend.
// tauri-specta generates bindings.ts from #[specta::specta] annotated commands.
// If a command is renamed/removed in Rust, this file will fail to compile.

import { commands } from "./bindings";
export { commands };
export type {
  AccessKeyInfo,
  AssumeRoleResult,
  BootstrapResult,
  BootstrapStep,
  CallerIdentity,
  ChatMessage,
  ChatModel,
  ChatRole,
  ClientSummary,
  ConfigInfo,
  CredentialAssessment,
  CredentialClass,
  CredentialSource,
  NewCredentials,
  Plan,
  PlanEntry,
  ScanResult,
  ScanStatus,
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
// Provisioner wrappers
// ---------------------------------------------------------------------------

export async function scanResources() {
  return unwrap(await commands.scanResources());
}

export async function previewPlan() {
  return unwrap(await commands.previewPlan());
}

export async function provision() {
  return unwrap(await commands.provision());
}

export async function destroy(): Promise<void> {
  unwrap(await commands.destroy());
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

// ---------------------------------------------------------------------------
// Chat wrappers
// ---------------------------------------------------------------------------

export async function listChatModels() {
  return unwrap(await commands.listChatModels());
}

export async function chatMessage(modelId: string, messages: import("./bindings").ChatMessage[]) {
  return unwrap(await commands.chatMessage(modelId, messages));
}

export async function acceptModelAgreement(modelId: string): Promise<void> {
  unwrap(await commands.acceptModelAgreement(modelId));
}