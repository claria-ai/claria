import { invoke } from "@tauri-apps/api/core";

export interface ConfigInfo {
  region: string;
  system_name: string;
  created_at: string;
  credential_type: string;
  profile_name: string | null;
  access_key_hint: string | null;
}

export interface CallerIdentity {
  account_id: string;
  arn: string;
  user_id: string;
}

export type CredentialSource =
  | { type: "inline"; access_key_id: string; secret_access_key: string }
  | { type: "profile"; profile_name: string }
  | { type: "default_chain" };

export async function hasConfig(): Promise<boolean> {
  return invoke("has_config");
}

export async function loadConfig(): Promise<ConfigInfo> {
  return invoke("load_config");
}

export async function saveConfig(
  region: string,
  system_name: string,
  credentials: CredentialSource
): Promise<void> {
  return invoke("save_config", { region, systemName: system_name, credentials });
}

export async function deleteConfig(): Promise<void> {
  return invoke("delete_config");
}

export async function validateCredentials(
  region: string,
  credentials: CredentialSource
): Promise<CallerIdentity> {
  return invoke("validate_credentials", { region, credentials });
}

export async function listAwsProfiles(): Promise<string[]> {
  return invoke("list_aws_profiles");
}
