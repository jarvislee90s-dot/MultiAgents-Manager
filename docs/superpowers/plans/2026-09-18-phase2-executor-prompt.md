# 二期首批（M6–M9）· Windows 执行机任务书（提示词）

> 本文即「给执行 Agent 的完整提示词」——自包含，执行机无需其他会话上下文。直接把本文全文发给 Windows 电脑上的 Agent 即可。

---

## 你的任务

在 **Windows 电脑**上执行 MAM 项目二期首批里程碑 **M6–M9（消息注入主线）**：M6 Windows 注入探测（立即执行）→ M7 macOS 终端注入全链 → M8 审批应答 → M9 Windows 注入实现（后三者等实施计划到达后依序执行）。你负责开发与可本机完成的实测；跨机实测项登记回传。全过程有独立评审（Mac 侧），诚实报告优于一切。

## 环境准备

```powershell
git clone https://github.com/jarvislee90s-dot/MultiAgents-Manager.git   # 已有 clone 则 git fetch
cd MultiAgents-Manager
git checkout feat/phase2-injection    # 二期工作基线分支（main + PR#68 全量预演合并，六门禁全绿）
pnpm install
```

分支上已有全部权威文档（**先读再干**，顺序如下）：

1. `docs/MASTER-PLAN.md` —— 项目宪法（D1–D18）。**任何实现与其冲突：停下上报，不得自行处置，更不得修改它**。
2. `docs/superpowers/specs/2026-09-18-phase2-message-injection-design.md` —— 二期总 spec v1.4：W1–W12 需求、裁决 1–17（**实施不得重议**）、里程碑 M6–M11、延后登记表。
3. `docs/superpowers/specs/2026-09-18-phase2-m6-m9-injection-design.md` —— 本批次设计 v2.1：T1–T4 任务分解、执行策略（T1 探测只 gate T4）、风险预登记。
4. `AGENTS.md` —— 工程契约：会话扫描预算红线（L1/L2/L3 + 预过滤反模式禁令）、`store.with` 持 DB 锁不可重入等。

## 阶段一（拿到任务立即执行）：M6 Windows 注入探测

按 `docs/superpowers/plans/2026-09-18-m6-windows-injection-probe.md` **逐任务执行**（该计划自包含：ConIn.ps1 助手脚本全文、探测矩阵、判定标准、报告模板）。要点提醒：

- 探测在**一次性测试会话**上做（临时项目目录、提示词只要求回复 `PROBE-OK`），绝不打扰真实工作会话；
- 送达判定只认「会话文件出现探测消息」地面真值；API 成功但未送达要如实区分；
- 双会话定向（不串台）是 GO 必要条件；
- 产出：`%USERPROFILE%\mam-probe\` 整目录（报告/台账/conin.log/证据/脚本），**打包回传给用户转交 Mac 侧评审**；
- 结论 GO / PARTIAL / NO-GO 三态照判定标准给出，**不阻塞阶段二开发**（只 gate M9 的实现范围）。

## 阶段二（依序 M7 → M8 → M9）：等实施计划到达

- 实施计划文件：`docs/superpowers/plans/2026-09-18-phase2-m7-m9-injection.md`（Mac 侧编写中）。定期 `git pull origin feat/phase2-injection` 检查；**该文件到达前不要凭设计文档自行开工写码**——跨任务接口契约（inject/ 模块签名、队列表 schema、路由表结构、端点形态）由计划锁定，自行发挥会返工。
- 计划到达后：严格按计划的 task 顺序与 TDD 步骤执行（写失败测试 → 跑失败 → 最小实现 → 跑过 → 提交），每 task 一个或一组 commit，commit message 描述任务与验证方式。
- M6 探测结论出来后：若 NO-GO / PARTIAL，按批次设计 T4 节的降级路径处理 M9（缩小范围或登记已知限制），并在台账记录裁决请求上报。

## 平台分工事实（重要，如实执行）

- **M7 的 macOS 三通道（tmux/iTerm2/Terminal.app）与 M8 的「终端按键生效」实机验证无法在 Windows 本机完成**：代码、单元测试、门禁、mock 测试照常全量做；实机出口项逐条登记到台账的「**回传 Mac 侧验收清单**」（写明：场景、前置条件、预期观察、涉及的 commit），不要伪造实测结论。
- M6/M9 的 Windows 实机项在本机完成（这就是你在 Windows 上的价值）。
- Rust 代码写跨平台时以 `cfg(target_os)` 分层，macOS 分支代码可写、标注「待 Mac 侧实机验证」。

## 门禁（每 task 完成必跑，全绿才算完）

```powershell
cd src-tauri; cargo fmt --check; cargo clippy --all-targets -- -D warnings; cargo test
cd ..; pnpm check; pnpm test; pnpm build:mobile
```

- Windows 上遇平台专属用例失败：按仓库既有 `cfg(unix)` 门控模式处理（main 有先例，见 reconcile_test），**不得删除用例**；如认为用例本身有环境依赖缺陷，修用例并在 commit 说明理由（参考 84cac2b 先例）。
- 一期零回归是硬约束：任何对 monitor/remote 既有行为的改动都要有明确理由与回归测试。

## 红线（违反即停）

1. **不 push、不建 PR、不动 main**——全部 commit 留在本地 `feat/phase2-injection` 工作副本上，合并决定永远属于用户；
2. **不修改** `docs/MASTER-PLAN.md` 与三份设计文档本体——发现冲突停下上报，由用户裁决；
3. 裁决 1–17 与批次设计已定事项**不得重议**（斜杠放行 / PIN 即信任 / 多行 `\n` 归一 / 插队 / 审批边界三档 / 由简到繁等）；
4. 写端点全部在既有 PIN 设备门禁之后，端点前缀沿 `/m/api/v1/` 惯例；
5. AGENTS.md 全部工程契约适用（扫描预算、store.with 非重入、测试不得依赖网络与真实 `~/.mam`）。

## 台账与回传

- 台账：`.superpowers/sdd/2026-09-18-phase2-injection-mainline/progress.md`（本地目录，仓库已 gitignore）——每 task 记：做了什么 / 门禁数字 / 偏离与原因 / 裁决请求 / 回传清单更新；
- 回传节点：① M6 报告打包；② 每 task 完成简报（commit hash 列表 + 门禁结果 + 发现）；③ 阶段性 handover（遇到阻塞 / 需要裁决 / 里程碑完成时）——格式参照：结论先行、已完成表、偏离与裁决请求、风险、下一步。

现在开始：先读四份权威文档，然后执行阶段一（M6 探测计划）。
