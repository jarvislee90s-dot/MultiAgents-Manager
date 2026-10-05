//! 供应商归因（D8）：优先读各工具配置文件/字段，尽力归因；三态来源标记必须可见。
//! 实测分布（说明书 §8.3）：ZCode / OpenCode / Kimi 实测；dsh 部分实测（28.3%）；
//! Codex / Claude Code 推断；WorkBuddy 完全不可得。归因不到 → 只按模型维度呈现。
use serde::Deserialize;

use super::model::{SourceKind, UsageSourceId};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProviderRule {
    pub prefix: String,
    pub provider: String,
}

#[derive(Debug, Deserialize)]
struct ProviderRulesDoc {
    #[serde(default)]
    rules: Vec<ProviderRule>,
}

/// 解析用户自维护规则（设置项 `providerMapRules`，JSON 文本）。非法 → 空表（不报错、不阻断采集）。
/// 返回表按 prefix 长度**倒序**，保证最长前缀优先命中。
pub fn parse_provider_rules(json: &str) -> Vec<ProviderRule> {
    if json.trim().is_empty() {
        return Vec::new();
    }
    let mut rules = serde_json::from_str::<ProviderRulesDoc>(json)
        .map(|d| d.rules)
        .unwrap_or_default();
    rules.retain(|r| !r.prefix.trim().is_empty() && !r.provider.trim().is_empty());
    // 按 prefix 长度**倒序**（最长前缀优先）。用 `sort_by_key` + `Reverse` 而非
    // `sort_by(|a, b| b.….cmp(&a.…))`：后者在 `-D warnings` 下触发
    // clippy::unnecessary_sort_by（本计划自己的门禁），与 D-05 / D-18 同先例，
    // **语义一字未改**（`sort_by_key` 同为稳定排序，仅比较方向等价改写）。
    rules.sort_by_key(|r| std::cmp::Reverse(r.prefix.len()));
    rules
}

/// 模型名内嵌供应商（kimi `ollama-cloud/glm-5.3-flash`）。
/// 返回 (供应商, 去掉前缀后的模型名)。无 `/` 或前缀为空 → ("", None)。
pub fn provider_from_model_prefix(model: &str) -> (String, Option<String>) {
    match model.split_once('/') {
        Some((prefix, rest)) if !prefix.trim().is_empty() && !rest.trim().is_empty() => {
            (prefix.trim().to_string(), Some(rest.trim().to_string()))
        }
        _ => (String::new(), None),
    }
}

/// 归因优先级：① 源内直接字段（实测）→ ② 模型名前缀（实测）→ ③ 用户规则（推断）→ ④ 未知。
/// **不做模糊匹配**（D9：模型名完全一致才算同一模型；规则是用户显式前缀，不算模糊）。
pub fn resolve_provider(
    _source: UsageSourceId,
    explicit: Option<&str>,
    model: &str,
    rules: &[ProviderRule],
) -> (String, SourceKind) {
    if let Some(p) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return (p.to_string(), SourceKind::Measured);
    }
    let (prefix, _) = provider_from_model_prefix(model);
    if !prefix.is_empty() {
        return (prefix, SourceKind::Measured);
    }
    for r in rules {
        if model.starts_with(&r.prefix) {
            return (r.provider.clone(), SourceKind::Inferred);
        }
    }
    (String::new(), SourceKind::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::usage::model::{SourceKind, UsageSourceId};

    /// Kimi：供应商内嵌在模型名里（`ollama-cloud/glm-5.3-flash`）→ 前缀即供应商（实测 D8）
    #[test]
    fn kimi_provider_comes_from_model_prefix() {
        assert_eq!(
            provider_from_model_prefix("ollama-cloud/glm-5.3-flash"),
            (
                "ollama-cloud".to_string(),
                Some("glm-5.3-flash".to_string())
            )
        );
        // 无前缀 → 无供应商（不猜）
        assert_eq!(
            provider_from_model_prefix("glm-5.3-flash"),
            (String::new(), None)
        );
        // 边界：前缀 / 模型名**任一为空**都不拆——`ollama-cloud/` 若返回 `Some("")`，
        // kimi 采集器会把模型名写成空串、明细行 model 与 provider 双空（D9 分组退化）
        assert_eq!(
            provider_from_model_prefix("ollama-cloud/"),
            (String::new(), None)
        );
        assert_eq!(
            provider_from_model_prefix("/glm-5.3-flash"),
            (String::new(), None)
        );
    }

    /// D8 三态：源内直接字段 = 实测；用户规则命中 = 推断；都无 = 未知
    #[test]
    fn provider_source_kind_three_states() {
        let rules =
            parse_provider_rules(r#"{"rules":[{"prefix":"deepseek-","provider":"volcengine"}]}"#);
        // 直接字段优先（实测）
        assert_eq!(
            resolve_provider(UsageSourceId::ZCode, Some("volcengine-plan"), "x", &rules),
            ("volcengine-plan".to_string(), SourceKind::Measured)
        );
        // 规则命中（推断）
        assert_eq!(
            resolve_provider(UsageSourceId::Claude, None, "deepseek-v4.1-flash", &rules),
            ("volcengine".to_string(), SourceKind::Inferred)
        );
        // 都无（未知）——不猜、不填 "unknown" 字符串
        assert_eq!(
            resolve_provider(UsageSourceId::WorkBuddy, None, "hy3", &rules),
            (String::new(), SourceKind::Unknown)
        );
        // 非法 JSON → 空规则，不报错
        assert!(parse_provider_rules("{not json").is_empty());
        // 最长前缀优先（避免 "deep" 抢走 "deepseek-" 的匹配）
        let long = parse_provider_rules(
            r#"{"rules":[{"prefix":"deep","provider":"A"},{"prefix":"deepseek-","provider":"B"}]}"#,
        );
        assert_eq!(
            resolve_provider(UsageSourceId::Claude, None, "deepseek-v4.1-flash", &long).0,
            "B"
        );
    }

    /// **偏差申报 D-1（新增用例，请裁决）**：任务书原用例集只锁住了三态链的第 ①③④ 档；
    /// 第 ② 档「模型名前缀 → **实测**」**零覆盖**——而它正是 kimi 全部供应商的来源（D8 实测）。
    /// 本用例把第 ② 档钉死，并顺带覆盖 `parse_provider_rules` 的两条过滤：
    /// 空 `prefix` 的规则会退化成 `model.starts_with("")` **恒真** → 把每一个模型都误判成
    /// 「推断」并盖上用户规则的供应商名（灾难性误归因），必须在解析期丢掉。
    #[test]
    fn provider_priority_second_step_is_measured_and_empty_rules_are_dropped() {
        let rules =
            parse_provider_rules(r#"{"rules":[{"prefix":"deepseek-","provider":"volcengine"}]}"#);
        // ② 模型名前缀 = 实测（kimi 实测形态：ollama-cloud/glm-5.3-flash）
        assert_eq!(
            resolve_provider(
                UsageSourceId::Kimi,
                None,
                "ollama-cloud/glm-5.3-flash",
                &rules
            ),
            ("ollama-cloud".to_string(), SourceKind::Measured)
        );
        // ① 强于 ②：两档同时在场时仍取源内直接字段
        assert_eq!(
            resolve_provider(
                UsageSourceId::Kimi,
                Some("zhipu"),
                "ollama-cloud/glm-5.3-flash",
                &rules
            ),
            ("zhipu".to_string(), SourceKind::Measured)
        );
        // 空 prefix / 空 provider 的规则必须被丢掉
        let dirty = parse_provider_rules(
            r#"{"rules":[{"prefix":"","provider":"A"},{"prefix":"deepseek-","provider":""},{"prefix":"x","provider":"B"}]}"#,
        );
        assert_eq!(dirty.len(), 1, "空 prefix / 空 provider 的规则必须被过滤");
        assert_eq!(
            resolve_provider(UsageSourceId::WorkBuddy, None, "hy3", &dirty),
            (String::new(), SourceKind::Unknown),
            "过滤后无规则可命中 → 未知档（不得被空 prefix 抢走）"
        );
        assert_eq!(
            resolve_provider(UsageSourceId::Claude, None, "x-model", &dirty),
            ("B".to_string(), SourceKind::Inferred)
        );
    }
}
