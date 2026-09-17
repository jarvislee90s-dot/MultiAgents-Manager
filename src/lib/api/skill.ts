import { invoke } from "@tauri-apps/api/core";
import type { ImportOutcome } from "@/types/extension";
export async function listRepoSkills() {
  return await invoke("list_repo_skills");
}
// 返回类型与 Rust commands::skill::ImportOutcome（serde camelCase）同步（Task 17）
export async function installSkill(
  sourcePath: string,
  name: string,
  overwrite = false
): Promise<ImportOutcome> {
  return await invoke<ImportOutcome>("install_skill", { sourcePath, name, overwrite });
}
export async function rescanSkills() {
  return await invoke("rescan_skills");
}
export async function assignSkillToSubagent(skillName: string, toolId: string, subAgentId: string) {
  return await invoke("assign_skill_to_subagent", { skillName, toolId, subAgentId });
}
