# Changelog

## [0.2.2] — 2026-07-08

### 新增
- 资源看板双视图（按分类 / 按工具）
- OpenClaw 第四工具支持（进程扫描 + state.json 解析 + Skill 映射）
- 原生资源扫描导入（Skill / MCP / Plugin）
- 预设组兼容性检查
- CLAUDE.md 项目架构文档
- Tauri DevTools 调试支持
- 浏览器渲染 Mock（tauri-mock.ts）

### 优化
- 会话监控扩展支持 OpenClaw APP 形态
- MCP 统一管理面板
- Plugin 文件/配置混合管理
- 11 个 Rust warnings（待清理）

### 打包
- macOS DMG | macOS .app | 自动更新包

---

## [0.2.1] — 2026-07-06

### 新增
- Plugin 管理模块（文件 + 配置类型）
- Layer 2/3 目录结构（工具级 + 子 Agent 级）
- 子 Agent 级预设组
- MCP 配置前端 UI
- 可配置提示音
- 截图 API（capture_window_screenshot + ScreenshotTool 组件）

### 优化
- preset 应用逻辑优化
- 设置页面集成
- 通知/快捷键配置

---

## [0.2.0] — 2026-07-05

### 新增
- 多工具会话监控看板（Claude Code / Codex CLI / OpenCode）
- 状态变更桌面通知 + Web Audio 提示音
- 终端快速跳转（iTerm2 / Terminal.app / tmux）
- 统一资源仓库管理（Skill 映射 + MCP 配置写入）
- 预设组一键切换（命名预设 + 托盘菜单）
- 子 Agent 级资源分配
- 系统托盘集成（状态指示 + 预设菜单）
- i18n 国际化（中文 / English）
- auto-update 自动更新
- 自定义 frameless 窗口（全平台拖拽/最小化/最大化/关闭）
- 暗色/亮色主题切换

### 架构
- AgentAdapter trait（Claude / Codex / OpenCode）
- 三层 symlink 映射（SSOT → Tool → SubAgent）
- Hook 优先状态检测 + 进程扫描回退
- SQLite 数据层（settings / extensions / presets / sessions）
- JSON / TOML / JSONC 多格式 MCP 配置写入
