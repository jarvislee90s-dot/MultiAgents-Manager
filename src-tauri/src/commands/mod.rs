// Tauri IPC 命令 - 按功能域拆分到子模块

pub mod data_management;
pub mod manifest;
pub mod mcp;
pub mod notification;
pub mod pet;
pub mod plugin;
pub mod preset;
pub mod resource;
pub mod screenshot;
pub mod session;
pub mod settings;
pub mod skill;
// 用量域（计划① Task 20）：6 条查询/设置命令（按字母序排在 skill 之后）
pub mod usage;

pub use screenshot::capture_window_screenshot;
pub use session::get_all_sessions;
