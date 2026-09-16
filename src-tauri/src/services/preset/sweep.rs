// 独占清扫（spec §5.1 步骤4）：差集 = 当前生效资源 − 预设项 − 常驻；
// MAM 资源走既有停用服务（含 Layer3 级联），原生技能走暂存引擎
use crate::database;

#[derive(Debug, Default)]
pub struct SweepPlan {
    /// 要停用的 MAM 资源 (extension_id, kind)
    pub disable_mam: Vec<(String, String)>,
    /// 要暂存的原生技能名（目录名，不带 skill- 前缀）
    pub stash_native: Vec<String>,
}

pub fn plan_sweep(tool_id: &str, keep: &[(String, String)]) -> SweepPlan {
    let mut plan = SweepPlan::default();
    let keep_ids: Vec<&str> = keep.iter().map(|(id, _)| id.as_str()).collect();
    for item in super::snapshot::scan_tool_state(tool_id) {
        if keep_ids.contains(&item.extension_id.as_str()) {
            continue;
        }
        if database::is_tool_resident(tool_id, &item.extension_id) {
            continue;
        }
        if item.origin == "mam" {
            plan.disable_mam.push((item.extension_id, item.kind));
        } else {
            // native 项 extension_id = "skill-<name>"
            let name = item
                .extension_id
                .strip_prefix("skill-")
                .unwrap_or(&item.extension_id);
            plan.stash_native.push(name.to_string());
        }
    }
    plan
}

/// 执行清扫：逐项 best-effort，失败进 failures 不阻断（spec §9，FR-6.32 部分成功）
pub fn execute_sweep(tool_id: &str, plan: &SweepPlan) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut disabled = Vec::new();
    let mut stashed = Vec::new();
    let mut failures = Vec::new();

    for (ext_id, kind) in &plan.disable_mam {
        // 停用分派统一走 toggle_ext（与 apply/restore 同源；kind 全部来自
        // scan_tool_state 的三前缀推导，未知 kind 臂不可达，论证见 toggle_ext 注释）
        match super::toggle_ext(ext_id, kind, tool_id, false) {
            Ok(()) => disabled.push(ext_id.clone()),
            Err(e) => failures.push(format!("{}: {}", ext_id, e)),
        }
    }

    if let Some(dir) = crate::adapter::primary_skill_dir(tool_id) {
        for name in &plan.stash_native {
            match super::stash::stash_native_skill(tool_id, name, &dir.join(name)) {
                Ok(()) => stashed.push(name.clone()),
                Err(e) => failures.push(format!("{}: {}", name, e)),
            }
        }
    }

    (disabled, stashed, failures)
}
