import { invoke } from "@tauri-apps/api/core";
export async function listRepoSkills() { return await invoke("list_repo_skills"); }
export async function installSkill(sourcePath: string, name: string) { return await invoke("install_skill", { sourcePath, name }); }
export async function rescanSkills() { return await invoke("rescan_skills"); }
export async function assignSkillToSubagent(skillName: string, toolId: string, subAgentId: string) { return await invoke("assign_skill_to_subagent", { skillName, toolId, subAgentId }); }
