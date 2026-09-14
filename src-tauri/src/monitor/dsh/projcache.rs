// projcache（会话投影缓存）：$DSH_HOME/storages/session_projcache/sessions/<id>.json
// 首选数据源（标题/活跃度/运行信号已预计算，M0 F5）；identity 5 字段强校验防"张冠李戴"

use serde_json::Value;
use std::path::Path;

use super::log::DshHeader;

#[derive(Debug, Clone, Default)]
pub struct ProjcacheView {
    pub title: Option<String>,
    pub last_prompt_at: Option<i64>,
    /// None = turnBoundary 行缺失（空闲会话）⇒ 无打开 turn；Some(true)=运行中
    pub open_turn: Option<bool>,
}

/// identity 校验（M0 F5 + 评审修正 #5）：formatVersion 必须存在且 == 日志 header 版本；
/// createdAt/cwd 一致；isSeeded 缺省 false。inheritedEventCount 双方都无来源可比对（header 无此字段），跳过。
pub fn identity_matches(identity: &Value, header: &DshHeader, header_version: i64) -> bool {
    let fv = match identity.get("formatVersion").and_then(|v| v.as_i64()) {
        Some(v) => v,
        None => return false, // 缺失即拒绝（session-projection-cache 语义）
    };
    if fv != header_version {
        return false;
    }
    let created_ok = identity.get("createdAt").and_then(|v| v.as_i64()) == header.created_at;
    let cwd_ok = identity.get("cwd").and_then(|v| v.as_str()) == header.cwd.as_deref();
    let seeded = identity
        .get("isSeeded")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let seeded_ok = seeded == header.is_seeded.unwrap_or(false);
    created_ok && cwd_ok && seeded_ok
}

/// 从 record JSON 提取视图（rows.<key>.val 动态形状）
pub fn view(record: &Value) -> Option<ProjcacheView> {
    let rows = record.get("record")?.get("rows")?;
    let row_val = |key: &str| rows.get(key).and_then(|r| r.get("val"));
    Some(ProjcacheView {
        title: row_val("title").and_then(|v| v.as_str()).map(String::from),
        last_prompt_at: row_val("sessionListMetadata")
            .and_then(|v| v.get("lastPromptAt"))
            .and_then(|v| v.as_i64()),
        open_turn: row_val("turnBoundary").map(|v| {
            v.get("openTurnStartSeq")
                .map(|s| !s.is_null())
                .unwrap_or(false)
        }),
    })
}

/// 读取 projcache 记录。坏记录/缺文件 → None（降级走日志源，backup-and-skip 同语义）
pub fn load(home: &Path, session_id: &str) -> Option<ProjcacheView> {
    let path = home
        .join("storages/session_projcache/sessions")
        .join(format!("{session_id}.json"));
    let text = std::fs::read_to_string(path).ok()?;
    let record: Value = serde_json::from_str(&text).ok()?;
    // domain version 门：[3,7] 之外弃缓存（M0：version 7 兼容 3–6）
    let ver = record.get("version").and_then(|v| v.as_i64())?;
    if !(3..=7).contains(&ver) {
        log::warn!(
            "dsh projcache: 未知 version {}，弃缓存（session {}）",
            ver,
            session_id
        );
        return None;
    }
    view(&record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::dsh::log::DshHeader;
    use serde_json::json;

    fn header() -> DshHeader {
        DshHeader {
            id: "session-1".into(),
            cwd: Some("/tmp/p".into()),
            parent_session: None,
            origin: None,
            delegation_depth: 0,
            created_at: Some(1000),
            is_seeded: Some(false),
            version: Some(3),
        }
    }

    #[test]
    fn identity_requires_format_version_match() {
        let h = header();
        // formatVersion 缺失 → 拒绝（M0 F5 陷阱：真实 home 12 条缺失）
        assert!(!identity_matches(
            &json!({ "createdAt": 1000, "cwd": "/tmp/p" }),
            &h,
            3
        ));
        // 不等 → 拒绝
        assert!(!identity_matches(
            &json!({ "formatVersion": 2, "createdAt": 1000, "cwd": "/tmp/p" }),
            &h,
            3
        ));
        // 全等 → 通过
        assert!(identity_matches(
            &json!({ "formatVersion": 3, "createdAt": 1000, "cwd": "/tmp/p", "isSeeded": false }),
            &h,
            3
        ));
        // createdAt/cwd 不一致 → 拒绝（张冠李戴防御）
        assert!(!identity_matches(
            &json!({ "formatVersion": 3, "createdAt": 9999, "cwd": "/tmp/p" }),
            &h,
            3
        ));
        assert!(!identity_matches(
            &json!({ "formatVersion": 3, "createdAt": 1000, "cwd": "/other" }),
            &h,
            3
        ));
        // isSeeded 缺省按 false（M0：isSeeded ?? false）
        assert!(identity_matches(
            &json!({ "formatVersion": 3, "createdAt": 1000, "cwd": "/tmp/p" }),
            &h,
            3
        ));
    }

    #[test]
    fn view_extracts_rows_and_missing_turn_boundary() {
        // rows.title.val / sessionListMetadata.val.lastPromptAt / turnBoundary.val.openTurnStartSeq
        let rec = json!({
            "record": { "identity": {}, "rows": {
                "title": { "ver": 1, "seq": 5, "val": "reply ok" },
                "sessionListMetadata": { "ver": 1, "seq": 6, "val": { "lastPromptAt": 123456 } },
                "turnBoundary": { "ver": 1, "seq": 6, "val": { "openTurnStartSeq": 4 } }
            }}
        });
        let v = view(&rec).unwrap();
        assert_eq!(v.title.as_deref(), Some("reply ok"));
        assert_eq!(v.last_prompt_at, Some(123456));
        assert_eq!(v.open_turn, Some(true));
        // 无 turnBoundary 行（空闲会话实测形态）⇒ open_turn=None ⇒ 视为无打开 turn
        let idle = json!({ "record": { "identity": {}, "rows": { "title": { "val": "t" } } } });
        assert_eq!(view(&idle).unwrap().open_turn, None);
    }

    #[test]
    fn load_reads_record_and_tolerates_missing() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let sess = home.join("storages/session_projcache/sessions");
        std::fs::create_dir_all(&sess).unwrap();
        std::fs::write(
            sess.join("session-1.json"),
            serde_json::to_string(&json!({
                "version": 7,
                "record": { "identity": {}, "rows": { "title": { "val": "hello" } } }
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            load(home, "session-1").unwrap().title.as_deref(),
            Some("hello")
        );
        assert!(load(home, "session-missing").is_none());
    }
}
