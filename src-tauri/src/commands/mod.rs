// Tauri IPC 命令 - 按功能域拆分到子模块

pub mod manifest;
pub mod mcp;
pub mod notification;
pub mod plugin;
pub mod preset;
pub mod resource;
pub mod screenshot;
pub mod session;
pub mod settings;
pub mod skill;

pub use screenshot::capture_window_screenshot;
pub use session::get_all_sessions;
