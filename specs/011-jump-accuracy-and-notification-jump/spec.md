# 功能规格说明：跳转准确性与通知窗联动

**功能分支**：`011-jump-accuracy-and-notification-jump`

**创建日期**：2026-08-25

**状态**：草稿

**输入**：用户实测反馈——三个 Claude 会话中两个终端会话跳转分不清（两个 WT 窗口标题完全相同 `✳ Claude Code`）；歧义时的候选列表包含了无关终端（opencode/codex/PowerShell 窗口）；通知卡显示时长 6 秒偏短、点击跳转逻辑与主界面重复实现。

## 根因与背景（实机取证）

1. **标题无区分度**：Windows Terminal 实测两个 claude 窗口标题逐字相同。spec 008 的 marker 层（hook 注入 `MAM:<session-id>` 到标题）未启用的根因是 **hook 注册假阳性**：`register_all_hooks`（`monitor/hooks.rs:184+`）用一个全局 `hooks_registered` 标志管理两个工具——任一工具注册成功即置 true，启动时整体跳过且永不核验文件实际状态；claude 的 `settings.json` 实际无 hooks 段但标志为 true，永不重试。spike 前提 B（OSC 序列写 `CONSOLE$`/tty 可达标题）已实测成立。
2. **候选未过滤**：`resolve_and_focus` 的 Ambiguous 分支返回该祖先进程的**全部**可见窗口，未按已有的标题打分结果筛选——无关终端混入候选列表。这是四层降级设计时的缺口。
3. 通知窗点击跳转与 SessionCard 各自实现了"调用 focus_session + 歧义候选"逻辑，存在重复且行为漂移风险。

## 用户场景与测试

### 用户故事 1 — 双 Claude 终端会话精确跳转（优先级: P1）

**验收场景**：

1. **给定** 两个终端分别跑同款工具的会话（如两个 claude，不同项目），**当** hook 生效后点击卡片，**则** 各自精确聚焦对应终端窗口（标题 `MAM:<session-id 前 8 位>` 匹配），不弹选择器
2. **给定** 卡片前缀与终端标题栏 marker，**则** 两者 8 位前缀可直接对读
3. **给定** claude 与 codex 的 hook 注册，**当** 应用启动，**则** `~/.claude/settings.json` 与 `~/.codex/hooks.json` 实际含 status-hook 配置（DB 标志与文件状态一致）；用户已有的其他 hooks 配置不被覆盖或删除

### 用户故事 2 — 歧义时候选列表只含相关终端（优先级: P1）

**验收场景**：

1. **给定** 打开多个终端（claude×2、opencode、PowerShell），**当** 点击 claude 卡片且无法唯一锁定（marker 未命中），**则** 候选列表优先仅显示标题含 claude 的窗口；无任何匹配时才展示全量候选（按打分排序）
2. **给定** 候选列表渲染，**则** 每项显示窗口标题与进程名，点选后聚焦

### 用户故事 3 — 通知卡更可用（优先级: P2）

**验收场景**：

1. **给定** 通知浮窗弹出，**当** 无交互，**则** 10 秒后自动隐藏（悬停保留、移开 5 秒隐藏）
2. **给定** 点击通知卡，**当** 跳转执行，**则** 行为与主界面点击同一会话卡片完全一致（同一实现，含歧义候选处理）
3. **给定** 主界面与通知窗任一处修改跳转逻辑，**则** 两处行为同步（共享实现）

## 设计

### 1. hook 注册修复（monitor/hooks.rs）

- 标志从全局单值改为**按工具**（`hooks_registered_claude` / `hooks_registered_codex`）
- 启动时**核验实际状态**而非信任标志：读取 `hook_config_path`，hooks 配置存在且包含 `status-hook.sh` 引用且脚本文件存在 → 跳过；否则执行合并式注册（保留用户已有 hooks 条目，仅追加缺失项），成功后更新对应标志。旧全局标志作废弃迁移（视为未注册，触发一次核验）

### 2. marker 注入启用（前置：修复 1 生效）

启用此前 spike 判定 no-go 时搁置的实现：`HOOK_SCRIPT` 追加标题注入行（`MAM:<session_id 前 8 位>` 写 `CONSOLE$`/tty），`ensure_hook_script` 改为总是重写（应用托管脚本，幂等）。spike 结论文档随实现更新。

### 3. 候选过滤（window/win32.rs）

`resolve_and_focus` 的 Ambiguous 分支：先用现有打分函数过滤出 `score > 0` 的窗口作为候选；空集时回退全量候选并按分数降序排序（`WindowCandidate` 增加 `score` 字段供前端排序展示）。

### 4. 通知卡时长与共享跳转（前端）

- `notification.tsx`：`armTimer(6000)` → `armTimer(10000)`；悬停移开 `armTimer(3000/5000)` 统一为 5000
- 抽共享 hook `useSessionJump()`（建议 `src/hooks/useSessionJump.ts`）：封装 `invoke focus_session（pid/sessionId/agentType/projectName） + ambiguous 结果返回`；`SessionCard` 与通知窗改用该 hook，各自的候选 UI（卡片弹层 / 通知窗内联列表）保持自有渲染

## 范围外

- UIA 内容匹配、Windows Terminal 标签页级定位（延续裁剪）
- macOS 侧行为（零回归约束：mod.rs macOS 分支不动）
- OpenCode/OpenClaw 的 hook 机制（两工具无 hook 能力，跳转消歧依赖候选过滤）

## 测试策略

- 纯函数单测：候选打分过滤逻辑（可从 resolve_and_focus 抽出测试）；hook 核验函数（给定配置文件内容 → 注册/跳过决策）
- 人工验证：用户故事 1-3 全部场景；重点为"双 claude + 一个 opencode + 一个空 PowerShell 同开"的矩阵测试（marker 命中 / 过滤后的候选列表 / 全量回退三级路径）
