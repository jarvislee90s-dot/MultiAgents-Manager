# Changelog

## [Unreleased]

## [0.2.0] - 2026-09-02
### Added
- Foxbell 桌宠：独立悬浮窗口（可置顶），状态卡片（红等待/黄运行/绿完成未读）+ 31 条状态语音 + 拖拽物理 + 完整右键菜单（大小三档/三场景动作绑定）；完成提示音接管与置顶时浮窗抑制；入口：看板设置/🦊 按钮/托盘/右键菜单（Tauri 暂无穿透 API，透明区遮挡下层点击）
- Kimi Code 第五工具支持：会话监控、Skill/MCP 管理、`KIMI_CODE_HOME` 数据目录重定向（自动回退旧版 `~/.kimi`）
- 资源管理增强：逐行批量启用/禁用、全类型搜索、扩展清单安装对话框与严格 semver 版本校验
### Fixed
- Kimi：修复轮次答完后状态灯一直卡黄（`turn.ended` 事件未映射）；`turn.ended` 现为唯一轮次结束信号，`usage.record` 不再映射以消除轮中进行中的瞬态误标
- Kimi：会话索引根目录一致性、越界索引项跳过、非 ASCII 会话标题截断 panic 等多项修复
- 修复 pull_request CI 因 `GITHUB_TOKEN` 缺少 `pull-requests: read` 权限而一直静默失败的问题
- 修复 Windows 终端跳转链（每进程 Codex 独立卡片、前台聚焦与逐层回退）
- 修复多工具上报同一会话时的重复卡片（按工具 + 会话 ID 聚合去重）
- macOS 启用 private API 修复透明窗口显示
### Changed
- monitor 解析器按工具拆分（claude/codex/kimi 等），抽出 cwd/JSONL/git 公共设施
- 适配器统一经中央工具注册表分发服务层调用
- Release 流程改为草稿优先：tag 触发云端编译后生成 draft release，人工验收后再发布

## [0.2.2] - 2026-07-08
### Added
- 资源管理看板双视图（按类型/按工具）
- 预设组一键应用/取消功能
- 兼容性检查对话框
- OpenClaw 第四工具支持
### Fixed
- 修复 TypeScript 未使用变量错误

## [0.1.0] - 2026-07-01
### Added
- 多 Agent 工具统一监控看板
- Skill/MCP/Plugin 三层映射架构
- 系统托盘预设菜单
- 状态变更桌面通知
- 终端快速跳转
