# 实现说明 — 监控解析层解耦 + Kimi Code 全链路接入

日期：2026-09-01 ｜ 分支：task3/finch

## 0. 交付物与验证结果总览

| 项 | 命令 | 结果 |
|---|---|---|
| macOS 编译 | `cd src-tauri && cargo check` | ✅ 通过（基线为 E0599 编译失败） |
| Rust lint | `cargo clippy --all-targets -- -D warnings` | ✅ 0 warning |
| Rust 测试 | `cargo test`（连跑 3 次） | ✅ lib 85 + dao 4 + linker 7，全绿且稳定 |
| 前端 lint | `pnpm lint` | ✅ 0 error（2 个 warning 为 ExtensionList.tsx 既有） |
| 前端格式 | `pnpm format:check` | ✅ |
| 前端构建 | `pnpm build` | ✅ |
| 前端测试 | `pnpm test` | ✅ 3 文件 11 测试 |
| i18n 键对齐 | `pnpm check:i18n` | ✅ 272 键 |

Linux 影响：macOS 修复所用 `macos-private-api` feature 在 tauri 内部由
`cfg(target_os = "macos")` 门控，非 macOS 为空操作；CI（ubuntu-latest）的
`cargo check/clippy/test` 路径不受影响。本机为 macOS（Darwin 25.6.0），
Linux 侧仅能以"feature 为空操作 + CI 同命令集"论证，未实机验证。

提交序列（语义化小步）：

```
b10dbb4 test(kimi): MCP write/remove roundtrip through the registry
7fbe08d feat(kimi): frontend integration — badge, icon, tool lists
2080bb8 feat(kimi): backend integration — adapter, session parser, process discovery
e24e9df refactor(adapter): central tool registry, dispatch service layer through it
d7d9d1c refactor(monitor): split monolithic parser.rs into per-tool parsers + shared infra
a34c3ae fix(macos): enable macOS-private-api for transparent windows + platform-aware cwd test
```

---

## 1. 前置门：macOS 编译修复

**现象**：基线 `cargo check` 在 macOS 失败——
`WebviewWindowBuilder::transparent()`（src-tauri/src/commands/notification.rs:97，
通知浮窗）在 Tauri v2/macOS 下由 `macos-private-api` feature 门控。
CI 只跑 Linux（`.github/workflows/ci.yml` backend job = ubuntu-latest），故长期未暴露。

**方案选择**（三选一并记录理由）：

1. ~~条件编译跳过 `.transparent(true)`~~ —— 通知浮窗丢失透明圆角，macOS 视觉行为回退，否决。
2. ~~仅 tauri.conf.json 加 `macOSPrivateApi: true`~~ —— tauri-build 报
   "features does not match the allowlist... or add the `macos-private-api` feature"，
   二者必须成对，否决。
3. **成对启用**（采用）：`tauri.conf.json` `app.macOSPrivateApi: true` +
   Cargo.toml `tauri` features 加 `macos-private-api`。主窗 `transparent: true`
   （tauri.conf.json 既有）同样依赖此 feature，一处修复两处受益。

**顺带修复的既有 macOS 测试失败**（基线 `cargo test` 即红，非本次引入）：
`cwd_equivalent_tests::separator_direction_and_trailing_are_equivalent` 无条件断言
Windows 专有的大小写不敏感行为（`cwd_equivalent("e:/llmproject/x", "E:\\LLMproject\\x\\")`），
在 macOS/Linux 必挂。按同文件兄弟用例 `case_rules_follow_platform` 的既有模式，
将该断言用 `cfg!(windows)` 平台化（Windows 行为不变）。

---

## 2. 监控解析层解耦

### 2.1 问题

`monitor/parser.rs`（约 37KB）同时混杂：Claude Code 与 Codex CLI 两套 JSONL
协议解析、GitHub URL 缓存、路径编解码、cwd 归一化、子 agent 计数等公共设施。
新工具接入被迫改这个热点文件，Claude/Codex 解析逻辑互相牵连。

### 2.2 拆分方案

`monitor/` 下按职责单一原则拆为 9 个模块（`parser.rs` 删除）：

| 模块 | 职责 | 使用方 |
|---|---|---|
| `cwd.rs` | cwd 归一化/等价比较（跨工具共享） | 全部 4+1 解析器 |
| `path_codec.rs` | Claude projects 目录名编解码（Claude 专用） | claude_parser |
| `git.rs` | GitHub URL 查询 + 进程内缓存 | claude/codex/kimi |
| `project.rs` | 项目名提取、cwd 形态校验 | claude/codex/kimi/jsonl |
| `jsonl.rs` | JSONL 尾部读取、cwd 提取、子 agent 计数、文件枚举 | claude/codex/kimi |
| `claude_parser.rs` | Claude message.role+content[] 协议 | — |
| `codex_parser.rs` | Codex type+payload 协议（rollout 文件） | — |
| `kimi_parser.rs` | Kimi session_index + wire 事件流（本次新增） | — |
| `opencode_parser.rs` / `openclaw_parser.rs` | 既有，仅改 import 指向 `cwd.rs` | — |

**测试零改动**：`path_tests`/`git_url_tests`/`normalize_cwd_tests`/
`cwd_equivalent_tests` 四组既有测试**逐字节随代码搬入对应模块**（断言一字未改），
`cargo test --lib` 71 → 71 全绿（其中 1 个平台断言按 §1 平台化）。

**一处行为保持的去重**：Claude/Codex 解析器尾部读取逻辑（512KB seek + 跳截断行 +
取末 500 行）原本逐字重复两份，提取为 `jsonl::read_recent_lines(path, max_lines)`
（按文件序返回，调用方 `.iter().rev()` 即得原"最新在前"遍历序）。语义等价性：
空文件/缺失文件/不足 500 行/超大文件四种边界与原实现逐一对照一致；
解析函数无直接单测（既有覆盖为活机扫描 `test_get_all_sessions`），
风险由等价性分析 + 活机扫描（见 §5）兜底。

### 2.3 中央工具登记表（adapter registry）

解耦的另一半：服务层原有 **8 处** `match tool_id { "claude" => …, "codex" => …,
"opencode" => …, "openclaw" => …, _ => … }`（mcp/skill/plugin×4/preset×2/
resource×3/detector），每加一个工具要逐个加 arm。收敛为 `adapter/mod.rs` 的
唯一登记处：

```rust
pub const TOOL_IDS: &[&str] = &["claude", "codex", "opencode", "openclaw", "kimi"];
pub fn adapter_by_id(tool_id: &str) -> Option<Box<dyn AgentAdapter>> { /* 唯一 match */ }
pub fn all_adapters() -> Vec<Box<dyn AgentAdapter>> { TOOL_IDS … filter_map(adapter_by_id) }
pub fn all_adapters_with_ids() -> Vec<(&'static str, Box<dyn AgentAdapter>)> { … }
```

8 处分发改走 registry（映射关系、迭代顺序逐点对照不变）；
`services/resource` 的 skill/plugin 扫描源改为由 adapter 派生
（值与原硬编码逐工具一致，新工具登记后自动纳入）。

**顺带修复 2 个既有 flaky 集成测试**（基线 `cargo test` 间歇红，已用
`git stash` 在未改代码的 d7d9d1c 上复现）：`linker_test.rs` 三个用例在
`Once` 共享的临时 HOME 里共用 `demo-skill` 夹具名/目录，并行执行下互相竞速
（"已存在同名资源"/内容被覆盖）。各自改用独立夹具名，纯测试改动，
生产代码零改动，修复后连跑 5 次全绿。

### 2.4 新旧行为对照证据

- 既有测试：71 个 lib 测试（含 4 组解析路径测试）断言零改动，全绿。
- 活机扫描：`test_get_all_sessions`（真实扫描本机）改动前后均为
  `Total: 0, Waiting: 0`（本机无 agent 进程运行），无回归。
- 顺序保持：`all_adapters()` 按 `TOOL_IDS` 序（= 原 vec 序），
  `get_all_sessions` 的会话聚合顺序不变。
- 前端：`pnpm build` + 11 测试全绿。

---

## 3. Kimi Code 接入决策记录

技术特征先经官方文档（moonshotai.github.io/kimi-code）核实，再与本机实机
`~/.kimi-code` 交叉验证（目录结构、session_index 字段名、wire 事件类型普查）。

### 3.1 数据布局（文档 + 实机一致）

```
$KIMI_CODE_HOME（默认 ~/.kimi-code，env 可重定向；早期版本 ~/.kimi 作回退）
├── config.toml              # TOML（providers/models/hooks…）
├── mcp.json                 # JSON，mcpServers 段（MCP 配置在这，不在 config.toml）
├── skills/                  # 用户级 skill 目录
├── plugins/managed/<id>/    # 插件（kimi.plugin.json 清单型）
├── session_index.jsonl      # 每行 {sessionId, sessionDir, workDir}
└── sessions/<workDirKey>/<sessionId>/
    ├── state.json           # {title, isCustomTitle, createdAt, updatedAt}
    └── agents/main/wire.jsonl   # 事件流（type 判别，time=epoch 毫秒）
```

### 3.2 会话识别方式

- **定位**：以 `session_index.jsonl` 的 `workDir` 与进程 cwd 归一化匹配
  （`monitor::cwd` 共享件）。`workDirKey = wd_<slug>_<sha256[:12]>` 不可逆算，
  索引是唯一稳定映射；索引缺失/为空 → 返回空（优雅降级，不误报）。
  `sessionDir` 实测为绝对路径，解析器同时兼容相对 sessions/ 与相对数据根。
- **一进程一卡**：同 workDir 多会话时取 wire.jsonl mtime 最新者
  （与 Codex Phase 1 同策略）；无有效 cwd 的进程回退最新未匹配会话（Phase 2）。
- **进程发现**：`find_processes_by_names(&["kimi"], …)`。Web Worker 子进程
  `kimi-code-worker` basename 不同名不会误匹配，且父链过滤会剔除同工具子进程。
- **标题**：优先 `state.json.title`（用户自定义/会话摘要，实测优于其他工具的
  8 位 id 前缀），回退 id 前 8 位保持卡片一致性。
- **跳转**：Kimi 为终端 TUI（CLI 形态），macOS 走既有 TTY 链路
  （iterm/Terminal.app/tmux，不依赖 agent_type），`jump_supported=true`；
  Windows 侧补 `TOOL_CLAIM_KEYWORDS`("kimi") 与 `running_projects` 面板反推。

### 3.3 状态判定规则（扫 wire.jsonl 尾部，取最新一条有效信号）

| wire 事件 | 状态 | 语义 |
|---|---|---|
| `context.append_loop_event` tool.call / step.begin | Processing（黄） | 工具执行中 / LLM 步进中 |
| `content.part`（part.type=text） | Processing（黄） | 回复流式输出中 |
| `content.part`（part.type=think） | Thinking（黄） | 模型思考中 |
| `step.end`（finishReason=tool_use） | Processing（黄） | 本步以工具调用结束，后续还有步骤 |
| `step.end`（end_turn）/ tool.result | （继续前扫） | 轮次边界/工具刚返回，非终态 |
| `turn.prompt` / `turn.steer` | Thinking（黄） | 用户刚提交输入 |
| `llm.request` / `permission.record_approval_result` / `goal.create`/`goal.update` | Processing（黄） | LLM 请求在飞 / 权限放行后续跑 / 目标模式轮次 |
| `usage.record` | Waiting（红） | 一轮结束（每轮最后一个事件），等待用户输入 |
| `turn.cancel` | Waiting（红） | 用户打断，回到输入态 |
| `context.append_message`（role=user） | Thinking（黄） | 用户消息 |
| `context.append_message`（role=assistant 纯文本） | Waiting（红） | 纯文本回复，轮次结束 |
| `context.append_message`（role=assistant 带 toolCalls） | Processing（黄） | 工具将执行 |
| `full_compaction.begin`（未见 complete/cancel） | Compacting（黄） | 压缩进行中 |
| 无任何信号 | 文件 60s 内有改动 → Processing，否则 Waiting | 与既有工具 file-age 兜底同语义 |

规则依据：本机 wire.jsonl 事件类型普查（`context.append_loop_event` 9176 条、
`usage.record` 1832、`llm.request` 621、`context.append_message` 361、
`turn.prompt` 190 等）+ 单会话样本结构（step.begin/content.part/step.end/
tool.call/tool.result/usage.record 的时序）。实测样本尾部
`step.end(end_turn) → usage.record` 即"轮次结束"形态，判 Waiting 正确。

### 3.4 MCP 下发格式（与任务书的偏差及依据）

任务书假设"按 TOML 格式下发到 ~/.kimi-code"，**官方文档核实结果不同**：
MCP 服务器声明在 `~/.kimi-code/mcp.json`（JSON，`mcpServers` 段，与 Claude 同构），
`config.toml` 仅放 `[mcp]` 超时等全局项。故 KimiAdapter：
`mcp_format() → Json`、`mcp_config_path() → ~/.kimi-code/mcp.json`，
复用既有 JSON writer（`mcpServers.<name>.{command,args,env}`），
零新增写入路径。决策依据：customization/mcp 官方文档原文
"MCP server declarations are configured in ~/.kimi-code/mcp.json … not in config.toml"。

### 3.5 其他决策

- **Hook**：`hook_supported() = false`。Kimi 支持 `[[hooks]]`（config.toml，
  PascalCase 事件，stdin JSON 含 hook_event_name/session_id/cwd），但现有注册器
  只写 Claude 风格 JSON 配置；TOML `[[hooks]]` 注册器为后续扩展。状态判定由
  wire.jsonl 尾部解析承担（与 opencode/openclaw 同档），不影响验收三要素。
- **Skill**：`skill_dirs() = [$KIMI_CODE_HOME/skills]`（官方用户级 skill 目录，
  SKILL.md frontmatter 与既有扫描器兼容）；`skill_dir_for_tool("kimi")` 经
  `kimi_home()` 保持 KIMI_CODE_HOME 与 adapter 同源。
- **Plugin**：`plugin_dirs() = [plugins/managed]`（官方安装位置），
  `plugin_config_paths() = []`（manifest 目录型，非配置段型）。
- **无新增运行时依赖**：Rust 侧仅用既有 serde/toml_edit/chrono/once_cell；
  前端零新依赖（图标为内联 SVG）。

### 3.6 优雅降级（无 Kimi 环境的机器）

- `~/.kimi-code` 不存在 → 进程发现空匹配、`detect_tools` 显示不可用、
  资源页显示空目录，均不报错（单测 `no_kimi_home_means_no_sessions` 等覆盖）。
- 索引/ wire 损坏行跳过（`malformed_index_lines_are_skipped`）。
- 前端 `ToolIcon` 对未知 toolId 回退 claude 图标；`AGENT_BADGE` 已含 kimi，
  SessionCard/通知浮窗/历史面板不会因缺 badge 崩溃。

---

## 4. "新增第六个工具"改动面（解耦可证明）

后端（以新增 Kimi 为实例）：

| 文件 | 改动 |
|---|---|
| `src-tauri/src/adapter/xxx.rs` | **新增** adapter 文件（~70 行） |
| `src-tauri/src/adapter/mod.rs` | 3 行：`pub mod xxx;` + `TOOL_IDS` + `adapter_by_id` 各 1 行 |
| `src-tauri/src/session/model.rs` | 1 行：`AgentType::Xxx` |
| `src-tauri/src/monitor/xxx_parser.rs` | **新增** 解析器（含单测） |
| `src-tauri/src/monitor/mod.rs` | 1 行：`pub mod xxx_parser;` |
| `src-tauri/src/monitor/process.rs` | ~3 行：`find_xxx_processes` |
| `src-tauri/src/commands/session.rs` | 1 行：Windows `running_projects` arm |
| `src-tauri/src/window/win32.rs` | 1 行：`TOOL_CLAIM_KEYWORDS`（Windows 跳转认领） |

**不需要动**：其他工具的 parser（claude/codex/opencode/openclaw）、
services 全部 match 站点（registry 分发）、linker、database。

前端：`constants.ts`、`types/session.ts`、`ToolIcon.tsx`、`BrandIcons.tsx`、
`agentBadge.tsx`、`audio.ts`、`settings.tsx`、4 个资源/预设视图、
`tauri-mock.ts`（各 1 行）+ 可选 badge 测试。

对比解耦前：加 Kimi 需改 parser.rs 热点 + 8 处服务层 match + 上述全部，
且 parser.rs 改动会牵连 Claude/Codex 解析。

---

## 5. Kimi 手动验收记录

### 5.1 已自动验证（本机，macOS）

**① 解析全链路（真实数据 + 伪造进程）**——临时集成测试（验证后已删除），
读取本机真实 `~/.kimi-code/session_index.jsonl`，伪造同 cwd 进程：

```
workDir: /Users/jarvis
parsed 1 session(s)
  id=session_9671e285-… title=Some("最新版本的 openclaw 我要怎么改它的模型？在哪个文件里改？")
  status=Waiting project=jarvis last_activity=2026-07-14T18:14:03.863Z jump=true role=Some("assistant")
test live_kimi_data_parses_end_to_end ... ok
```

真实 state.json 标题、wire 时间戳（epoch 毫秒 → ISO）、Waiting 状态
（尾部为 usage.record）均正确。

**② MCP 映射全链路**——`mcp_write_and_remove_roundtrip` 单测：
`write_mcp("kimi","demo",…)` → `~/.kimi-code/mcp.json` 出现
`mcpServers.demo.{command,args}`；`remove_mcp` 后清除。✅

**③ 状态判定**——14 个 kimi 单测覆盖：turn.prompt→Thinking、
usage.record→Waiting、tool.call/step.begin→Processing、think part→Thinking、
step.end(tool_use)→Processing、turn.cancel→Waiting、full_compaction.begin→
Compacting、空 wire 兜底、cwd 不匹配不出卡、损坏索引容忍。✅

**④ 既有工具无回归**——85 lib 测试全绿（71 既有断言零改动）+
活机扫描 `Total: 0` 与基线一致。

### 5.2 建议的人工验收命令（交互环境）

```bash
# ① 看板状态色：启动 kimi 会话后
pnpm tauri:dev    # 看板应出现 Kimi Code 卡片：
                  # 提问后→黄(Thinking/Processing)，回答完→红(Waiting)
# ② 点击跳转：点击卡片 → 聚焦对应终端（TTY 链路）
# ③ MCP 映射：资源页 → MCP → 选一个服务器 → 启用给 Kimi Code
cat ~/.kimi-code/mcp.json   # 应出现 mcpServers.<name>
```

交互截图：未在本环境采集（无显示会话交互），以 5.1 的命令输出为准。

---

## 6. 已知限制 / 后续工作

1. **Kimi hook 注册**：TOML `[[hooks]]` 注册器未实现（`hook_supported=false`），
   状态实时性依赖 3s 轮询 + wire 尾部解析；需要时在 `monitor/hooks.rs` 增加
   TOML writer 并置 true（stdin 载荷字段已确认兼容现有 status-hook.sh）。
2. **权限等待态**：Kimi 等待用户批准工具调用时，wire 尾部通常是触发审批的
   tool.call/step → 显示 Processing（黄）。Claude 对 AskUserQuestion 有专门
   Waiting 映射；Kimi 的权限请求事件类型未在文档中公开，未做特判。
3. **子 agent 计数**：Kimi swarm 子 agent 未计数（`active_subagent_count=0`）。
4. **Linux 实机**：仅 macOS 实机验证；Linux 论证依据为 feature 空操作 +
   CI 同命令集，未实机跑过。
