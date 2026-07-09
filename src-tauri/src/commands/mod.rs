// Tauri IPC 命令 - 按功能域拆分到子模块

pub mod session;
pub mod resource;
pub mod preset;
pub mod skill;
pub mod mcp;
pub mod plugin;
pub mod settings;
pub mod screenshot;
pub mod manifest;

pub use session::get_all_sessions;
pub use screenshot::capture_window_screenshot;

use tauri::{Builder, Runtime};

/// 注册所有命令到 Tauri builder
pub fn add_commands<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    let builder = session::add_commands(builder);
    let builder = resource::add_commands(builder);
    let builder = preset::add_commands(builder);
    let builder = skill::add_commands(builder);
    let builder = mcp::add_commands(builder);
    let builder = plugin::add_commands(builder);
    let builder = settings::add_commands(builder);
    let builder = screenshot::add_commands(builder);
    let builder = manifest::add_commands(builder);
    builder
}
