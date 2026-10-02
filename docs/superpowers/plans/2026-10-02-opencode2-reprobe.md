# opencode 2.x 注入规格复验计划（大版本漂移触发 · win-console-inject-probe 技能执行）

> 日期：2026-10-02 · 性质：**纯探测，零代码修改**
> 触发条款：AGENTS.md「工具大版本升级后的注入规格复验」——Phase C 执行期 opencode 批内自升 **1.18.32 → 2.0.22**，形态变为服务架构，`AttachConsole` 报 err=5/6（无可附加控制台），Windows 注入腿不可达，create E2E 的 opencode 腿被卡（Phase C 交付简报遗留 #1）。
> 在案基线：opencode **1.18.32** 线上 create 全链两次实机通过（`src-tauri/tests/create_e2e.rs` 文件头台账）——降级路径有实证，本探测不重跑 1.x。
> 执行方：Windows 段（主）在本机项目仓库下新会话执行；Mac 段在另一台 Mac 拉取本分支执行（量力）。
> 产出：规格定案表 + **GO / NO-GO** 结论（归档 `research/refs/phase2-消息注入/2026-10-02-opencode2-复验.md`）。

## 0. 纪律（先读再动）

1. **必读项目技能** `.agents/skills/win-console-inject-probe/SKILL.md` 并遵守八条铁律（追加式日志带 run-id / 只碰自建会话 / taskkill 清场 / FreeConsole 复位 / 结论不超证据……）。缺脚本按 `scripts/README.md` 接口重写；屏读脚本可用既有 `ConOut.ps1`（副本在 `%USERPROFILE%\mam-probe-m6r\evidence\create-probe\20260927-100552\scripts\`）。
2. 证据基座：`%USERPROFILE%\mam-probe-m6r\evidence\create-probe\opencode2-<run-id>\`；探测独立于 MAM 应用（不需要 MAM 运行）。
3. 不修代码、不 commit、不 push；npm 全局只动 opencode（探测结束**不自行恢复版本**——报告末尾写明当前装的版本，恢复与否由主线裁决）。
4. **自动化优先**：起窗/进程树/attach/模式读取/注入/db 对账全部脚本化。**只在两处调用用户**：① PrintWindow 抓不到 2.x 窗口、形态判定需要肉眼佐证（截图或口述现象）；② ConOut 屏读读不到新 TUI 文本、锚文案取证需要人工抄录。其余一律自动，不要中途找人。

## 1. Windows 段（Q1→Q5 顺序执行，前问定后问）

### Q1 形态判定（10 分钟）

`conhost.exe cmd /k opencode`（全新临时目录 `%TEMP%\mam-oc2-<run-id>`）起窗 →
- 进程树取证：进程名 / 父子链 / 命令行（Get-CimInstance）；窗口标题；PrintWindow 截图（脚本抓不到窗口就是结论之一，如实记录）。
- 结论三选：**(a)** 控制台 TUI（旧行为）／**(b)** 前台服务、无 TUI 交互界面（仅日志输出）／**(c)** server + 独立 UI（web 端口 / desktop 窗口 / 自动开浏览器）。
- 同时记录：`opencode --version`、npm 包名与版本、启动 stdout 首屏全部文本。

### Q2 可附加性穷举（10 分钟）

对 Q1 进程树里**每个**候选 pid（opencode 主进程 / node 子进程 / 任何 console 子进程）逐一 `AttachConsole`（ConIn.ps1 基座）：
- 记录每个 pid 的 err 码（0=成功；5=拒绝访问；6=句柄无效=该进程无控制台）；成功者再读 `GetConsoleMode`。
- err=5/6 的**复现条件**写清楚（是否稳定、是否所有候选皆然）。

### Q3 注入规格（仅当 Q1=a 且 Q2 存在可附加 pid；否则跳过并在报告写「不可达」）

按技能快路径 P1–P5 执行并填表：
- **P1 模式采样**：idle / busy 两态各 2 次 → 族判定（A 族 0x0208 / B 族 0x01F0 / **其他 = 新族**，新族则读 `references/methodology.md` 全量流程）。
- **P2 短消息真值**：39 字符探针 + 回车 → 会话存储命中对账（见 Q4 落点）。
- **P4 回车形态**：VK 回车 vs vk=0 字符回车各一次（新 TUI 可能改框架，旧结论不作数）。
- **P3 长度阶梯（缩量）**：200 / 2000 两档 × 3 取样（10k 档等主线放行后再补）。
- **CREATE 场景锚**：idle 输入框锚文案逐字（旧锚 `Ask anything` 是否仍在）；更新横幅形态（如有）。
- 慢消费者判定：首档注入后占用率曲线（`observe-occ.ps1`）——1.18.31 是已知慢消费者（~70 事件/s），2.x 可能变。

### Q4 会话存储落点（并入 P2 对账，30 分钟）

- 2.x 会话存到哪：`~/.local/share/opencode/opencode.db` 仍在？表结构（session/message/part 三表）变没变？
- **三件套拷贝法**（db+wal+shm）是否仍可行；物化时延（首条用户消息 → 落盘）实测。
- MAM 发现层兼容性**只评估不修改**：对照 `src-tauri/src/monitor/opencode_parser.rs` 的读路径键（session.directory 等），列出不兼容点（若有）。

### Q5 降级对照

不重跑——引用 create_e2e 台账（1.18.32 两次全链在案）即可。

### 汇总

写 `spec-draft.md` 到证据目录：**GO**（2.x 可注入，附全规格表）/ **NO-GO**（注入通道结构性不可达，附证据链）。NO-GO 时给主线三个选项各一行利弊：① pin 1.18.32；② opencode 移出 create 四家（改「另评」，候选目录来源照旧）；③ 等 2.x TUI 回归再复验。最终消息返回简报（Q1–Q4 各一行结论 + GO/NO-GO + 证据路径 + 需人工项清单若有）。

## 2. Mac 段（量力，M1–M3）

**对齐要求（硬性）**：clone 本仓库 → `git checkout probe/opencode2-reprobe`；**opencode 版本与 Windows 段探测报告所载版本完全一致**（`npm i -g opencode-ai@<该版本>` 后 `--version` 核对写进报告）。其余工具（claude/codex/kimi）**不要求统一版本**——异构版本由锚点账本同槽多文案机制承接，且保留对照价值。

- **M1** 形态判定（同 Q1）：Terminal.app `do script` 起窗 + 进程树 + 现象；若 2.x 无控制台 TUI，Mac 直接同结论 NO-GO 记录（不必深挖）。
- **M2**（仅 M1=TUI 时）注入规格：三路现成通道（tmux capture-pane / iTerm `text` / Terminal `contents of tab`）读写可行性 + P1–P3 缩量矩阵 + 存储落点（Q4 同构）。
- **M3** 汇总报告**全文返回**（结构：环境基线含 opencode 版本核对 / M1–M2 结论 / 规格表或 NO-GO 证据 / 清理说明 / 遗留）。全程不 commit 不 push、只碰自建临时目录、结束清进程。

## 3. 版本统一的边界说明（写给两台机器的执行者）

本探测**只统一 opencode 双机同版**（探测对象，结论才有可比性）；claude/codex/kimi 各机保持现状，版本指纹如实记入报告即可——锚点账本设计上就是多版本并存的（同槽多文案，版本号仅诊断备注）。
