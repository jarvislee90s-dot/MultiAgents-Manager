pub mod connection;
pub mod dao;
pub mod migration;
pub mod schema;

// 重新导出 DAO 模块，保持 crate::database::xxx 引用兼容
pub use dao::agent_tool;
pub use dao::extension;
pub use dao::preset;
pub use dao::session;
pub use dao::settings;

// 重新导出公共类型（保持 crate::database::Type -> crate::database::Type 兼容）
pub use dao::agent_tool::SubAgentRecord;
pub use dao::extension::{AssignmentRecord, ExtensionRecord};
pub use dao::preset::{PresetItemRecord, PresetRecord};

// 重新导出公共函数
pub use dao::agent_tool::{
    enabled_tool_ids, ensure_tool_rows, get_tool_enabled, list_sub_agents, set_tool_enabled,
};
pub use dao::extension::{
    delete_assignments_for, delete_extension, disable_subagent_assignment, insert_extension,
    list_all_assignments, list_assignments, list_extensions, upsert_assignment,
    upsert_assignment_with_subagent,
};
pub use dao::preset::{
    create_preset, delete_preset, get_preset_items, list_presets, record_preset_application,
    record_preset_application_subagent,
};
pub use dao::session::{cleanup_stale_sessions, update_session_status};
pub use dao::settings::{get_setting, set_setting};
pub use dao::unread::{
    clear_tool as clear_unread_tool, delete as delete_unread, list as list_unread_sessions,
};
pub use dao::unread::UnreadSessionRecord;

/// 初始化数据库（兼容旧 store::init() 调用）
pub fn init() {
    connection::init();
    if let Ok(conn) = connection::open() {
        let _ = migration::migrate(&conn);
    }
    // 种子工具启用行（全部 enabled=1，幂等；缺行视为启用）
    dao::agent_tool::ensure_tool_rows();
}
