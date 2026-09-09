import { invoke } from "@tauri-apps/api/core";

export interface ValidationError {
  field: string;
  message: string;
  code: string;
}
export interface ValidateResult {
  valid: boolean;
  manifest?: unknown;
  errors?: ValidationError[];
}
/** 卸载结果：needsConfirmation=true 表示 SSOT 技能仍被 ~/.agents/skills 直链引用，未做任何变更 */
export interface UninstallOutcome {
  needsConfirmation: boolean;
}

export async function validateManifestPath(path: string): Promise<ValidateResult> {
  return await invoke("validate_manifest", { path });
}
export async function installResource(path: string): Promise<void> {
  return await invoke("install_resource_from_manifest", { path });
}
export async function uninstallResource(
  kind: string,
  name: string,
  force?: boolean
): Promise<UninstallOutcome> {
  return await invoke("uninstall_resource", { kind, name, force });
}
export async function getStoreIndex(): Promise<unknown> {
  return await invoke("get_store_index");
}
