import { invoke } from "@tauri-apps/api/core";

export interface ValidationError { field: string; message: string; code: string; }
export interface ValidateResult { valid: boolean; manifest?: unknown; errors?: ValidationError[]; }

export async function validateManifestPath(path: string): Promise<ValidateResult> {
  return await invoke("validate_manifest", { path });
}
export async function installResource(path: string): Promise<void> {
  return await invoke("install_resource_from_manifest", { path });
}
export async function uninstallResource(extId: string, kind: string): Promise<void> {
  return await invoke("uninstall_resource", { extId, kind });
}
export async function getStoreIndex(): Promise<unknown> {
  return await invoke("get_store_index");
}
