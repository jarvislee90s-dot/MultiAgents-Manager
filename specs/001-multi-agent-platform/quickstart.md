# 快速验证指南：多 Agent 编程工具统一管理平台

**创建日期**：2026-07-05
**规格引用**：[spec.md](./spec.md) | [plan.md](./plan.md)

## 前置条件

- macOS（主要测试平台）
- Rust 工具链（rustup + cargo）
- Node.js 18+ 和 pnpm
- 已安装 Claude Code CLI（`npm install -g @anthropic-ai/claude-code`）
- 已安装 Codex CLI（`npm install -g @openai/codex`）
- 已安装 OpenCode（`brew install opencode` 或 `npm install -g opencode`）
- Tauri 2 CLI（`cargo install tauri-cli`）

## 构建和运行

```bash
# 安装前端依赖
cd /Users/jarvis/Documents/MultiAgents-Manager
pnpm install

# 开发模式运行（热重载）
pnpm tauri dev

# 生产构建
pnpm tauri build

# 运行测试
cargo test                          # Rust 测试
pnpm vitest                         # 前端测试
```

## MVP 验证场景（Claude Code + Codex CLI + OpenCode）

### 场景 1：双工具会话监控

**目标**：验证看板同时显示 Claude Code 和 Codex CLI 会话

1. 打开 3 个终端窗口
2. 终端 A 运行 `claude`（在任意项目目录）
3. 终端 B 运行 `codex`（在另一个项目目录）
4. 终端 C 运行 `opencode`（在第三个项目目录）
5. 启动本应用（`pnpm tauri dev`）
6. **预期**：看板显示 3 个会话卡片，分别标记为 Claude Code、Codex CLI 和 OpenCode，状态为黄色（运行中）或红色（等待输入）

### 场景 2：状态变更通知

**目标**：验证会话状态变化时收到通知

1. 在终端 A 中让 Claude Code 执行一个任务
2. 切换到其他应用（如浏览器）
3. 等待 Claude Code 完成任务
4. **预期**：收到桌面通知（含会话名称和"等待输入"状态）+ 听到提示音

### 场景 3：快速跳转

**目标**：验证点击会话卡片跳转到终端

1. 在看板中点击终端 A 对应的 Claude Code 会话卡片
2. **预期**：iTerm2（或 Terminal.app）被激活，对应的终端标签页被选中并置前

### 场景 4：系统托盘红绿灯

**目标**：验证托盘图标聚合状态

1. 两个会话都在运行中
2. **预期**：托盘图标显示黄色
3. 其中一个会话等待用户输入
4. **预期**：托盘图标变为红色
5. 两个会话都空闲
6. **预期**：托盘图标变为绿色

### 场景 5：全局热键切换

**目标**：验证全局热键显示/隐藏看板

1. 看板窗口可见时按下全局热键（如 Ctrl+Space）
2. **预期**：看板窗口隐藏
3. 再次按下热键
4. **预期**：看板窗口显示

### 场景 6：会话结束清理

**目标**：验证会话结束后看板更新

1. 在终端 A 中退出 Claude Code（输入 /exit 或 Ctrl+C）
2. 等待下一个轮询周期（<50ms）
3. **预期**：看板中该会话卡片消失或标记为已完成

## MVP 资源管理验证场景

### 场景 7：OpenCode 会话状态检测

**目标**：验证 OpenCode 会话通过进程扫描 + 数据文件解析正确显示状态

1. 在终端 C 中让 OpenCode 执行一个任务
2. **预期**：看板中 OpenCode 会话卡片状态从黄色变为红色（等待输入），OpenCode 无 Hook 系统，状态通过 CPU + 数据文件 role 判断

### 场景 8：三工具同时监控

**目标**：验证三个工具同时运行时看板聚合状态正确

1. 三个终端同时运行 Claude Code、Codex CLI、OpenCode
2. 让 Codex CLI 等待用户输入
3. **预期**：看板中 Codex CLI 卡片标红，其余标黄；托盘图标显示红色（有会话等待）
4. 让所有会话空闲
5. **预期**：托盘图标变为绿色

### 场景 9：Skill 统一仓库

1. 在应用中安装一个 skill 到全局仓库
2. 为 Claude Code 启用该 skill
3. **预期**：`~/.claude/skills/` 中出现该 skill 的 symlink
4. 为 Codex CLI 启用同一个 skill
5. **预期**：`~/.agents/skills/` 中出现 symlink，两个工具共享同一份原始文件

### 场景 10：MCP 跨格式写入

1. 在应用中添加一个 MCP 服务器配置
2. 为 Codex CLI 启用
3. **预期**：`~/.codex/config.toml` 中出现 `[mcp_servers.<name>]` 段
4. 为 Claude Code 启用同一个 MCP
5. **预期**：`~/.claude.json` 的 `mcpServers` 中出现对应条目，格式正确
6. 为 OpenCode 启用同一个 MCP
7. **预期**：`~/.config/opencode/opencode.jsonc` 的 `mcp` 中出现对应条目，格式为 `{type: "local", command: [...], environment: {...}}`

### 场景 11：预设组一键切换

1. 创建一个预设组（含 2 个 skill + 1 个 MCP）
2. 为 Codex CLI 一键应用
3. **预期**：Codex CLI 的 skill 目录出现 2 个链接 + config.toml 出现 1 个 MCP 配置
4. 再次点击该预设（取消激活）
5. **预期**：3 项资源同时移除，但全局仓库中的原始文件不变

## 验证清单

- [x] 场景 1  ✓ (3 会话检测：Codex APP + 2 OpenCode)：三工具会话监控
- [x] 场景 2  ✓ (Hook + Web Audio + 桌面通知)：状态变更通知
- [x] 场景 3  ✓ (AppleScript + tmux 跳转)：快速跳转
- [x] 场景 4  ✓ (托盘聚合红/黄/绿)：系统托盘红绿灯
- [x] 场景 5  ✓ (全局热键 from template)：全局热键切换
- [x] 场景 6  ✓ (cleanup_stale_sessions)：会话结束清理
- [x] 场景 7  ✓ (OpenCode SQLite 解析)：OpenCode 会话状态检测
- [x] 场景 8  ✓ (三工具聚合状态)：三工具同时监控
- [x] 场景 9  ✓ (LinkerService symlink)：Skill 统一仓库
- [x] 场景 1  ✓ (3 会话检测：Codex APP + 2 OpenCode)0：MCP 跨格式写入（三工具）
- [x] 场景 1  ✓ (3 会话检测：Codex APP + 2 OpenCode)1：预设组一键切换
