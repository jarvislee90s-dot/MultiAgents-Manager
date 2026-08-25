# 功能规格说明：状态判定与通知触发修复

**功能分支**：`010-status-accuracy-and-notification`

**创建日期**：2026-08-25

**状态**：草稿

**输入**：用户实测反馈与实机取证——任务结束后通知重复弹出（绿↔黄横跳）、OpenCode 空闲时状态抖动、Codex App 任务结束最长 5 分钟不变绿。根因定位（未改码）完成，见下。

## 根因记录（实机取证）

1. **`determine_status`（`monitor/status.rs:127-135`）**：`assistant` 分支的 `has_tool_use || file_recently_modified → Processing` 中，"文件最近修改"信号有权力否决"assistant 已完成（无 tool_use）"的明确完成信号——任务结束后文件被收尾写入刷新 mtime，在年龄窗口内（CLI 60s / **Codex App 300s**）被拉回黄，窗口滑出才变绿。5 分钟延迟 = App 的 300s 阈值精确对应；绿黄横跳与变绿拖延是同一表达式的两个症状。
2. **`determine_opencode_status`（`opencode_parser.rs:349`）**：`cpu > 5.0 → Processing`——CPU 为瞬时采样，任务结束后 OpenCode 的后台活动（GC/索引/文件监听）瞬时破 5% 即绿→黄，回落又变绿。
3. **通知层（`useNotification.ts:114-121`）**：颜色变化即弹；文件顶部注释承诺"同会话同状态 5 秒去重"但实现无时间维度——抖动的每次翻转都弹窗。轮询机制本身未变。

## 用户场景与测试

### 用户故事 1 — 任务结束状态及时收敛（优先级: P1）

任务结束后，会话卡片应及时变绿并稳定保持，不再回跳。

**验收场景**：

1. **给定** Codex App 会话任务结束（最后一条 assistant 回复写入），**当** 下一轮轮询完成，**则** 状态变绿（等待时长 ≤ 2 个轮询周期），且 5 分钟内不回跳黄色
2. **给定** CLI 类（claude/codex）任务结束，**当** 下一轮轮询完成，**则** 状态变绿（≤ 2 个轮询周期）
3. **给定** 任务仍在进行（assistant 消息含 tool_use），**则** 状态保持黄不受影响
4. **给定** 流式回复场景（entry 追加写入中），**则** 不会因修复而误判提前变绿（JSONL entry 原子写入语义下，assistant 纯文本 entry 出现即代表本轮回复完成）

### 用户故事 2 — 空闲会话不抖动（优先级: P1）

**验收场景**：

1. **给定** OpenCode 任务结束进入空闲，**当** 其进程后台活动导致 CPU 瞬时波动，**则** 状态保持绿，不出现绿黄横跳
2. **给定** OpenCode 真正在处理用户请求，**则** 状态正常判黄（防抖不得引入漏判）
3. **给定** 用户在空闲会话发起新任务，**则** 状态在轮询周期内变红/黄（开始提示不受影响）

### 用户故事 3 — 无状态变化不打扰（优先级: P1）

**验收场景**：

1. **给定** 任务已结束且无新输入，**当** 持续观察 10 分钟，**则** 不再出现任何通知弹窗
2. **给定** 同一会话颜色在 5 秒内因任何原因重复翻转，**则** 仅弹一次（时间维度去重兜底）
3. **给定** 用户主动开始新任务（绿→红/黄）或任务完成（黄→绿），**则** 弹窗行为保持现状（三种触发均保留）

## 设计

### 1. `determine_status` 完成信号优先（status.rs）

`assistant` 分支改为：`has_tool_use（且非用户输入类工具）→ Processing`；**否则 → Idle（不再被 file_recently_modified 拉回）**。`file_recently_modified` 保留两个用途：`user` 分支不变；`_`（无有效消息）兜底分支保持现状。Codex App 的 300s 阈值随之自然降权（仅兜底分支可达），无需单独调整数值。

### 2. `determine_opencode_status` CPU 防抖（opencode_parser.rs）

两层收紧：① **CPU 不再覆盖 assistant 已完成**——`last_role == Some("assistant")` 时 CPU 信号被忽略（进程在后台活动而会话已回复完毕，CPU 无意义）；② CPU 升级 Processing 的阈值从 5% 提高至 15%（瞬时采样噪声余量）。`user` 分支与 60s 活跃窗口逻辑不变。

### 3. 通知层时间去重（useNotification.ts）

`prevStatuses` 的值从 `string` 扩展为 `{ status, color, at }`；颜色变化触发通知前检查：同会话**同目标颜色**距上次通知 < 5 秒则跳过（真正实现文件头注释承诺的语义）。弹窗触发条件（任意颜色变化）与三方向触发保持现状；声音的"仅黄→绿响"规则归 spec 012 的音效系统，不在本 spec。

## 范围外

- 提示音/音效系统（spec 012）
- hook 注册修复与 marker（spec 011）
- 轮询间隔调整

## 测试策略

- 纯函数单测：`determine_status` 修改后的分支矩阵（assistant+tool_use / assistant 纯文本+文件新 / assistant 纯文本+文件旧 / user / 兜底）；opencode 防抖逻辑（若抽纯函数）
- 人工验证：三个用户故事的全部验收场景（Codex App 5 分钟场景、OpenCode 空闲观察 10 分钟为核心）
