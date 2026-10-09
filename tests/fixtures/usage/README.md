# `tests/fixtures/usage/` —— 真实后端返回的契约对账夹具（计划② Task 15 步骤 1）

## 这是什么

四份**真实后端返回**的 JSON，人肉从 兔维斯 dev 窗口的 DevTools 控制台抓下来，供
`tests/pet/usage-contract-parity.test.tsx` 的**第一组**逐字对账：真实 wire 形状 ↔
`src/types/usage.ts` 的 TS 镜像（契约 §2 的冻结件）。

| 文件 | 命令 | JS 入参（逐字） |
|---|---|---|
| `dashboard.last7d.json` | `usage_dashboard` | `{ range: { preset: "last7d" }, groupBy: "tool" }` |
| `records.project.json` | `usage_records` | `{ range: { preset: "last7d" }, groupBy: "project", filters: {} }` |
| `settings.json` | `usage_get_settings` | 无参 |
| `collect.json` | `usage_collect` | `{ force: true }` |

**现状：尚未抓取**（本目录的入库件是这份 `README.md` 与 `.gitignore`——后者以 `*.json` 规则
机械地挡住它们进索引）。四份 JSON 齐备前，对账测试第一组 5 条整组
**skip**（这是「还没联调」的正确状态，不是失败），第二组 1 条常绿（拿同源 mock 钉住「必需
字段表 ↔ 前端类型镜像」这件事本身）。抓齐后应变为 **5 passed + 1 skipped**。

抓完**不要入库**：这四个文件是真机用量数据，留在本地即可（`git add` 时不要带上它们）。

## 怎么抓（DevTools 四步）

前置：`~/.tuvis/tuvis.db` 四表有数据；设置页「用量统计」**总开关为开**（关闭时后端在读游标前就
早退，`usage_collect` 会返回空 `sources`，抓了也没用）。

1. `pnpm tauri:dev` 起开发模式，在 兔维斯 主窗口打开 DevTools 控制台（macOS 上 ⌥⌘I）。
2. **先跑一次采集**（顺序不能反：`usage_dashboard` / `usage_records` 的 `collectedAt` 取自本进程
   最近一次采集结果；先采集，它才不是 `0` 这个「尚未采集」哨兵）：

   ```js
   copy(JSON.stringify(await window.__TAURI_INTERNALS__.invoke("usage_collect", { force: true }), null, 2))
   ```

   控制台把 JSON 写进剪贴板后，回到仓库根目录：

   ```bash
   pbpaste > tests/fixtures/usage/collect.json
   ```

3. 另外三份同法（`copy(...)` → `pbpaste > tests/fixtures/usage/<文件名>`）：

   ```js
   copy(JSON.stringify(await window.__TAURI_INTERNALS__.invoke("usage_dashboard", { range: { preset: "last7d" }, groupBy: "tool" }), null, 2))
   copy(JSON.stringify(await window.__TAURI_INTERNALS__.invoke("usage_records", { range: { preset: "last7d" }, groupBy: "project", filters: {} }), null, 2))
   copy(JSON.stringify(await window.__TAURI_INTERNALS__.invoke("usage_get_settings"), null, 2))
   ```

   > 控制台没有 `copy()` 时：先 `JSON.stringify(x, null, 2)` 打印，再在输出上右键 →「Copy
   > object」；或直接手工复制控制台里的对象文本。落盘后请确认真的是 JSON（首字符 `{`）。

4. 校验：`pnpm test tests/pet/usage-contract-parity.test.tsx` → 期望 **5 passed + 1 skipped**。

## 抓完怎么用

- 第一组（5 条）从此参与每次全量回归：`dashboard` 12 个必需字段 + 行/趋势/环比/本会话/工作小结/
  可得性深断言；`records` 5 个必需字段 + 「卡片维度 ≠ 卡内行维度」（D6/D7）；`settings` 8 项；
  `collect` 7 源状态；外加隐私字面量扫描。
- 第一组若**红**：按报错字段名改**消费点**（`src/types/usage.ts` 与 `t()` 用法），**不改后端也不改
  契约**——契约是冻结件，漂移一定是单方面违约，此时停下报告（brief 步骤 3）。**唯一例外**是抓取
  质量问题（该档无数据、档位抓错、`collectedAt === 0`、总开关关闭）——按报错提示重抓即可。

## 禁止（spec P8，逐条都有人命）

- **不得写入会话正文 / prompt 原文 / 代码片段 / 附件路径 / 任何密钥、token、账号**。对账测试用
  字面量扫描兜底：`lastMessage` / `prompt` / `apiKey` / `Bearer ` 命中即红。若某个**项目名**恰好
  含 `prompt` 之类的子串而误红，**不要手工删改 fixture 内容**——如实上报，由计划负责人裁决。
- 不得为了让测试变绿而修改这四份 JSON 的内容（「改 fixture 迁就断言」= 伪造证据）。
- 不得把这份目录里的 `*.json` 提交进库（含 `git add -f`）；本目录的入库件只有 `README.md` 与
  `.gitignore`（后者以 `*.json` 规则机械地挡住它们进索引）。
