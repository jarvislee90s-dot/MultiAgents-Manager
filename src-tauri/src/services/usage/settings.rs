//! 用量设置读写（settings KV，键 `usage.settings`，值是 `UsageSettings` 的 JSON）。
//! 校验在 `merge_patch` 里做（越界一律 `usage-settings-invalid`，不静默夹取）。
use super::error::UsageError;
use super::model::{UsageSettings, UsageSettingsPatch};
use super::provider::parse_provider_rules;
use crate::database::{get_setting, set_setting};

pub const SETTINGS_KEY: &str = "usage.settings";

pub fn parse_settings(raw: Option<&str>) -> UsageSettings {
    raw.and_then(|s| serde_json::from_str::<UsageSettings>(s).ok())
        .unwrap_or_default()
}

pub fn merge_patch(
    cur: &UsageSettings,
    patch: &UsageSettingsPatch,
) -> Result<UsageSettings, UsageError> {
    let mut next = cur.clone();
    if let Some(v) = patch.enabled {
        next.enabled = v;
    }
    if let Some(v) = patch.mini_bar_range {
        next.mini_bar_range = v;
    }
    if let Some(v) = patch.mini_bar_tool_rows {
        if !(1..=7).contains(&v) {
            return Err(UsageError::new(
                "usage-settings-invalid",
                "浮窗分工具条数须在 1–7",
            ));
        }
        next.mini_bar_tool_rows = v;
    }
    if let Some(v) = patch.detail_retention_days {
        if !(1..=3650).contains(&v) {
            return Err(UsageError::new(
                "usage-settings-invalid",
                "明细保留期须在 1–3650 天",
            ));
        }
        next.detail_retention_days = v;
    }
    if let Some(v) = patch.collect_interval_min {
        if !(1..=1440).contains(&v) {
            return Err(UsageError::new(
                "usage-settings-invalid",
                "采集兜底间隔须在 1–1440 分钟",
            ));
        }
        next.collect_interval_min = v;
    }
    if let Some(v) = &patch.provider_map_rules {
        if !v.trim().is_empty() && parse_provider_rules(v).is_empty() {
            return Err(UsageError::new(
                "usage-settings-invalid",
                "供应商映射规则必须是 {\"rules\":[{\"prefix\":…,\"provider\":…}]} 形态的 JSON",
            ));
        }
        next.provider_map_rules = v.clone();
    }
    if let Some(v) = &patch.export_quote {
        next.export_quote = v.clone();
    }
    if let Some(v) = &patch.export_pose {
        if v.trim().is_empty() {
            return Err(UsageError::new(
                "usage-settings-invalid",
                "分享图姿态不得为空串（随机请用 random）",
            ));
        }
        next.export_pose = v.clone();
    }
    Ok(next)
}

/// 单测注入的设置覆盖（**仅 `#[cfg(test)]`**，评审 C5）：让调度层/采集器的单测
/// **完全不碰数据库**——旧版几个用例经 `save()` 写的是开发机真实 `~/.mam/mam.db`。
/// 用**全局静态量**而不是 `thread_local`：`concurrent_calls_share_one_run` 会在 4 个
/// 子线程里调用 `collect_with` → `load()`，thread_local 传不进去。
#[cfg(test)]
static SETTINGS_OVERRIDE: once_cell::sync::Lazy<std::sync::Mutex<Option<UsageSettings>>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(None));

/// 单测专用：注入一份设置（不落库、不读库）；传 `None` 复位。
#[cfg(test)]
pub fn set_override_for_test(s: Option<UsageSettings>) {
    *SETTINGS_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = s;
}

pub fn load() -> UsageSettings {
    #[cfg(test)]
    if let Some(s) = SETTINGS_OVERRIDE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
    {
        return s; // 单测注入优先：本分支在生产构建里不存在
    }
    parse_settings(get_setting(SETTINGS_KEY).as_deref())
}

/// 连接注入版（单测/查询层用）：**不触碰全局 DB**，避免单测污染真实 `~/.mam/mam.db`
pub fn load_from_conn(conn: &rusqlite::Connection) -> UsageSettings {
    parse_settings(crate::database::dao::settings::get_setting_conn(conn, SETTINGS_KEY).as_deref())
}

pub fn save(s: &UsageSettings) {
    if let Ok(json) = serde_json::to_string(s) {
        set_setting(SETTINGS_KEY, &json);
    }
}
