# 批次戊 · Stage 1 探测执行提示词（交执行 agent，2026-09-22）

> 用法：`---` 之后全文作为执行 agent 任务指令。探测阶段**零产品代码改动**；定案表回来经用户过目后才出 Stage 2 编码计划。

---

你是 MAM 项目「批次戊 · Stage 1 终端交互探测」执行 agent。工作目录
E:\LLMproject\Github\mam-worktree2（分支 feat/phase2-injection，基线=开工时 HEAD，设计文稿提交
abffb9f 之后）。

任务书（唯一 SSOT）：docs/superpowers/plans/2026-09-22-phase2-batch-e-design.md——重点读
§0 用户裁决清单（裁11-15，尤其 **裁15 证据规范**）、§3 Stage 1 探测操作计划（A–G 七项，
每项自带定案项与产出，**这就是你的任务清单，不需要再拆 task**）。

必读档案（开工前读完）：
- research/refs/phase2-消息注入/2026-09-22-键序大词典.md（每项探测的已知状态与〔待测〕格——
  你的工作是把这些格变成〔实测〕并回填）
- docs/superpowers/plans/2026-09-22-phase2-batch-e-gap-matrix.md（§1 八个根因——你探测的就是
  它们的定案面）
- .agents/skills/win-console-inject-probe/SKILL.md + references/known-families.md + scripts/README.md
  （八条铁律与脚本用法）

## 执行方式

探测 A/B/C 一组、D/E/F/G 一组，两组可并行派发子代理（每个探测一个子代理，自起会话自取证据）；
你作为主控汇总。也可按你判断重分组，原则：同一终端的探测尽量归同一子代理（省环境搭建）。

## 铁律（违反即作废重做）

1. **零产品代码改动**——src/ 与 src-tauri/src/ 一个字节都不动；探测脚本与档案落
   research/refs/phase2-消息注入/ 与 %TEMP% 探测目录。
2. 八条铁律：只碰自己 spawn 的进程/控制台；临时目录带 run-id；日志追加式；每次注入记
   BOOL/实写数/错误码；≥2 取样（争议 ≥5）；结束 taskkill 清场复核零存活；**不碰用户自有会话**；
   **结论不超证据**。
3. **裁15 证据规范**：每一步附「注入了什么键 → 屏读逐行原文」对照表（read_screen_window /
   ConOut.ps1 逐行 dump），不给纯截图结论。这是硬要求，报告与档案都要有。
4. **Esc 类注入只按一次**+屏读确认后行动（用户裁定：多按可能退出会话/丢队列）。
5. Git Bash 编排：MSYS_NO_PATHCONV=1；含中文 .ps1 存 UTF-8 BOM + CRLF；AttachConsole 前先
   FreeConsole。
6. 族形态：claude/kimi/opencode=A 族（字符流+VK 回车）；codex=B 族（一律 VK，禁裸 ESC 字符流）。
7. 探测中的争议（同输入不同结果）如实记〔矛盾〕并加测至 ≥5 取样，不擅自裁决。

## 产出（探测收口交付）

1. **七份探测档案**：research/refs/phase2-消息注入/2026-09-22-戊探<A-G>-<主题>.md（每份：环境/
   逐步键序-屏读对照表/定案表/证据索引/未测面留白）。
2. **键序大词典回填**：对应行的〔待测〕/〔矛盾〕→〔实测〕+档案引用（原文不删，追加修订注记）。
3. **差距矩阵 v2**：gap-matrix.md 增「探测定案」列（或标注），未定案项如实保留。
4. **定案表汇总（给用户过目的一页纸）**：每家每交互一个键序行（注入序列→预期屏读→回执/落账判据），
   单独成节附在任一档案或独立文件。
5. README.md 索引同步（research 目录）。
6. 探测 G 的回填建议（拍数/窗宽三处：常量+钉值+探针文档）**只写建议不改代码**。

台账：.superpowers/sdd/2026-09-18-phase2-injection-mainline/progress.md 文末追加「批次戊 Stage 1」
节（探测×档案×结论一行一记录）。全部产出单 commit（`docs(probe-e): 批次戊 Stage 1 探测——…`；
research/ 不入库，实际入库的可能只有 gap 矩阵/词典在 research 也全部本地——若全部不入库则
commit 为空操作，台账本地留档即可，报告说明）。不 push。

## 报告格式

Status / A–G 每项一行结论（定案了什么，引用档案）/ 争议与未定案项清单 / 词典与矩阵回填摘要 /
定案表全文（主控要转用户过目）/ 清场确认。
