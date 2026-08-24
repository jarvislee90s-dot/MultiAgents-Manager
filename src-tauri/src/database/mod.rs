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
pub use dao::extension::{AssignmentRecord, ExtensionRecord, NativeExtensionRecord};
pub use dao::preset::{PresetItemRecord, PresetRecord};

// 重新导出公共函数
pub use dao::agent_tool::list_sub_agents;
pub use dao::extension::{
    disable_subagent_assignment, insert_extension, insert_native_extension, list_all_assignments,
    list_assignments, list_extensions, list_native_extensions, mark_native_imported,
    upsert_assignment, upsert_assignment_with_subagent,
};
pub use dao::preset::{
    create_preset, delete_preset, get_preset_items, list_presets, record_preset_application,
    record_preset_application_subagent,
};
pub use dao::session::{cleanup_stale_sessions, update_session_status};
pub use dao::settings::{get_setting, set_setting};

/// 初始化数据库（兼容旧 store::init() 调用）
pub fn init() {
    connection::init();
    if let Ok(conn) = connection::open() {
        let _ = migration::migrate(&conn);
    }
}
