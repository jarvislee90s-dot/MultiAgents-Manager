# Release Notes

## v0.2.2 (2026-07-08)

### 🚀 新增

- **资源看板双视图**: 新增按资源分类（Skills/MCP/Plugins）和按工具（Claude/Codex/OpenCode/OpenClaw）两种视图
- **OpenClaw 支持**: 新增第四工具支持 — 进程扫描、会话解析（state.json）、Skill 映射
- **原生资源扫描导入**: 扫描本机已安装的 Skill/MCP/Plugin，一键导入统一仓库
- **兼容性检查**: 预设组应用前检查哪些资源可用于目标工具
- **CLAUDE.md**: 添加项目架构文档，方便 AI 工具理解代码库
- **Tauri DevTools**: 启用 `devtools` feature，支持 Safari Web Inspector 调试
- **浏览器渲染 Mock**: 创建 `tauri-mock.ts`，支持在浏览器中预览应用 UI

### 🔧 优化

- **会话监控**: 支持 OpenClaw 桌面 APP 形态的会话检测
- **MCP 管理**: 统一面板管理各工具的 MCP 服务器配置
- **Plugin 管理**: 文件/配置混合管理模式，支持 OpenClaw

### 📦 打包

- macOS DMG: `MultiAgents Manager_0.2.2_aarch64.dmg` (5.4MB)
- macOS .app: `MultiAgents Manager.app`
- 自动更新: `.app.tar.gz` (updater 就绪)
