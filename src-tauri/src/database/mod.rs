pub mod schema;
pub mod connection;
pub mod migration;
pub mod dao;

// 重新导出 DAO 模块，保持 crate::database::xxx 引用兼容
pub use dao::session;
pub use dao::extension;
pub use dao::preset;
pub use dao::settings;
pub use dao::agent_tool;

// 重新导出公共类型（保持 crate::database::Type -> crate::database::Type 兼容）
pub use dao::extension::{ExtensionRecord, AssignmentRecord, NativeExtensionRecord};
pub use dao::preset::{PresetRecord, PresetItemRecord};
pub use dao::agent_tool::SubAgentRecord;

// 重新导出公共函数
pub use dao::session::{update_session_status, cleanup_stale_sessions};
pub use dao::settings::{get_setting, set_setting};
pub use dao::extension::{
    insert_extension, list_extensions, list_assignments, list_all_assignments,
    upsert_assignment, upsert_assignment_with_subagent, disable_subagent_assignment,
    insert_native_extension, list_native_extensions, mark_native_imported,
};
pub use dao::preset::{
    create_preset, list_presets, delete_preset, get_preset_items,
    record_preset_application, record_preset_application_subagent,
};
pub use dao::agent_tool::list_sub_agents;

/// 初始化数据库（兼容旧 store::init() 调用）
pub fn init() {
    connection::init();
}
