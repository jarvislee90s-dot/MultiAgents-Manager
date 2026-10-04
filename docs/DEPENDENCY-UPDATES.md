# 依赖升级与月度批处理指南

> 适用对象：需要处理 dependabot PR、或改动依赖相关配置的人（含 AI agent）。
> 相关文件：`.github/dependabot.yml`、`.github/workflows/ci.yml`、`scripts/check-tauri-parity.mjs`、`package.json`（`engines`）、`src-tauri/Cargo.toml`（`rust-version`）。

## 1. 更新是怎么来的

| 事实 | 说明 |
|---|---|
| 节奏 | `schedule.interval: monthly` = **每月 1 日**运行一次 |
| 改配置会立刻触发 | 修改 `.github/dependabot.yml` 并合并后，dependabot 会立即重新检查一次（2026-10-03 实证：16:45 合并配置，16:47–16:52 就开出了 #102–#105） |
| 安全更新不受节奏约束 | `schedule`、`open-pull-requests-limit`、`multi-ecosystem-groups` 都只作用于 **version updates**；安全更新随时单独开 PR，需要及时处理 |
| 一 PR 覆盖两个生态 | `multi-ecosystem-groups.deps` 把 npm 与 cargo 合进同一个 PR —— 这是为了让 tauri 的 Rust crate 与 `@tauri-apps/*` NPM 包天然同批（见 §4） |
| 手动触发检查 | Insights → Dependency graph → Dependabot → **Check for updates**（改完配置想立刻验证时用） |

## 2. 门禁与分支保护

`ci.yml` 共五个 job，其中 **4 个是必需检查**（main 的分支保护）：

| 检查 | 内容 | 必需 |
|---|---|---|
| `Detect changed areas` | paths-filter 判定跑哪些 job | 否 |
| `Tauri version parity` | 只读两个锁文件，约 10 秒；Rust crate ↔ NPM 包 major.minor 必须相等 | ✅ |
| `Frontend Checks` | `pnpm install --frozen-lockfile` / lint / format:check / build / test | ✅ |
| `Backend Checks` | cargo fmt / check / clippy `-D warnings` / test + `x86_64-pc-windows-gnu --all-targets --features marker-helper` 交叉 check+clippy | ✅ |
| `Build verification` | `pnpm tauri build --no-bundle` | ✅ |

关键语义（均出自 GitHub 官方文档）：

- **`skipped` 也算通过**：必需检查的状态只要是 `successful` / `skipped` / `neutral` 就不阻塞合并。所以只改文档的 PR（前端/后端 job 全 skipped）照样能合。
- **`strict`（Require branches to be up to date）已开启**：分支必须与 main 同步才能合。含义是**每次合的都是「CI 在当时的 main 上验过的合并结果」**——这正是让"月度批处理"安全的机制，代价是每合一个 PR，其余 PR 会变成 `behind`，需要先更新分支。
- `enforce_admins: false`：仓库所有者保留人工绿色通道（真需要时可绕过门禁）。

## 3. 月度批处理流程

```bash
# 1. 列出待处理 PR，按 CI 结论分类
gh pr list --state open --json number,title,statusCheckRollup

# 2. 全绿的：逐个合并；每合一个，其余会变 behind，先更新分支再等新 head 绿
gh pr merge <N> --squash
gh api -X PUT repos/jarvislee90s-dot/MultiAgents-Manager/pulls/<下一个N>/update-branch
#    等新 head 的必需检查全绿（约 10–15 分钟）后再合下一个

# 3. tauri 两半（若 parity 报「成对不一致」）：必须同批合，见 §4

# 4. 红的：按 §4 的症状表判断——需要写迁移的单独排期，不要在批处理里现场写

# 5. 收尾：验证 main 自身
gh workflow run ci.yml --ref main

# 6. 在本文档 §7「批次记录」追加一行
```

## 4. 症状 → 处置

| 症状 | 原因 | 处置 |
|---|---|---|
| `Tauri version parity` 红，输出里成对出现「Rust 侧 ≠ NPM 侧」 | 同一批 tauri 升级被拆成两个 PR（npm 侧 / cargo 侧各一半），单独合任何一个都必红 | 两个 PR **同批合**：先合一个 → 对另一个 `update-branch` → 等 parity 转绿 → 再合 |
| `Tauri version parity` 红，只有一对不一致且是 patch 之外的漂移（如 `opener 2.6.0 ≠ 2.7.0`） | `@tauri-apps/*` 的 `^2` 这类松范围在 rebase/重解析锁文件时漂到了更新的 minor | 要么把 NPM 侧锁回与 Rust 相同的 minor，要么把 Rust 侧升到同一 minor（两边必须一致） |
| `Backend Checks` 红：`error[E…]` 编译错误 | major 升级带来的 API 破坏（历史案例：`rand 0.10` 删除 `thread_rng`/`RngCore`；`windows 0.61` 句柄指针化；`sysinfo 0.39` 删除 `RefreshKind::new`） | 立"迁移任务"单独做；E0599 这类"方法找不到"会**掩盖后续类型错误**（windows 那次 CI 报 16 个、实际 21 个），必须本地 `cargo check` 迭代到干净 |
| `Frontend Checks` 红：`typescript-eslint does not support TS X` | ESLint 工具链尚未支持该 TS 大版本（上游阻塞） | 对该依赖 `@dependabot ignore this major version`，等上游支持后 `unignore` |
| `Build verification` 红：`tsconfig.json(...) TS5102: Option 'baseUrl' has been removed` | TS 7 移除 `baseUrl` | 迁移 `tsconfig.json` 的 `baseUrl` + `paths` 写法（后端/前端构建都要验） |
| PR 显示 `behind` / 合并按钮灰 | `strict` 要求分支最新 | `gh api -X PUT repos/jarvislee90s-dot/MultiAgents-Manager/pulls/<N>/update-branch` |
| 改完 `dependabot.yml` 后**一个 PR 都不来** | 配置不合法（会让**所有**更新静默停摆） | Insights → Dependency graph → Dependabot 看有没有报错；确认写错就立刻回滚该文件 |

## 5. dependabot 评论命令（在 PR 里发评论）

| 命令 | 作用 |
|---|---|
| `@dependabot rebase` | 重新基于最新 main 生成分支（会重跑 CI） |
| `@dependabot recreate` | 重建 PR（**覆盖**对该 PR 分支的任何手工改动） |
| `@dependabot ignore this major version` | 关闭本 PR，并且不再提这个大版本（直到你手动升或 `unignore`） |
| `@dependabot ignore this minor version` / `… patch version` | 同上，作用于更小粒度 |
| `@dependabot ignore this dependency` | 完全不再为这个依赖开 PR |
| `@dependabot ignore <依赖名> major version` | 只忽略指定依赖的该大版本（组 PR 里按包裁剪时用这个） |
| `@dependabot unignore <依赖名>` / `@dependabot unignore *` | 清除已记录的 ignore 条件并重开 PR |
| `@dependabot show <依赖名> ignore conditions` | 列出该依赖当前的全部 ignore 条件 |

> 组 PR 里想"先拿一部分"：对不打算现在做的包逐个 `@dependabot ignore <依赖名> major version`，dependabot 会重开一个不含它们的 PR。

## 6. 环境下限（不要凭感觉写）

| 下限 | 声明位置 | 为什么是这个值 |
|---|---|---|
| Node.js **≥ 22.22.2** | `package.json` → `engines.node` | `jsdom 30` 的 engines 要求（测试环境）；CI 用 `node-version: 22` 浮动到最新 22.x 才恰好满足 |
| Rust **≥ 1.89** | `src-tauri/Cargo.toml` → `rust-version` | 代码实际用到 `File::unlock`（1.89 稳定，`src/linker/mod.rs`）与 `u64::is_multiple_of`（1.87 稳定，`src/services/pet/import.rs`）；依赖侧底线更低（`sha2 0.11` 要求 1.85） |
| pnpm **11** | CI `pnpm/action-setup` | 与锁文件版本一致 |

声明 MSRV 的附带收益：会激活 clippy 的 `incompatible_msrv`（默认 allow，一旦声明即 warn），**以后再误用超版本 API 会在 CI 直接暴露**。2026-10-03 就是这么发现真实 MSRV 是 1.89 而不是 1.85 的。

## 7. 批次记录

- **2026-10-03**：季度改月度 + 加 `Tauri version parity` 门禁 + 删自动合并工作流（PR #101）；#86+#91（tauri 2.12 两半）成对合并；#87 zstd / #88 sha2 / #92 @types/node / #93 eslint / #94 react-i18next / #95 jsdom 逐个合并；#89 rand 与 #90 windows 因需迁移代码，改由 #99 / #100 带迁移重做后合并。当日 main 五项门禁全绿。
- **待办**：见 §8。

## 8. 已知缺口

- **`release.yml` 的平台矩阵**：PR 阶段只验 Ubuntu + `windows-gnu` **只编译不链接**；macOS universal 与 Windows MSVC 只在打 tag 时构建。批处理后建议 `gh workflow run release.yml` 干跑一次。
- **majors 无法再用 PR 隔离**：多生态组把大版本与小版本放进同一个 PR（换来"两半天然同批"）。代价用 §5 的 ignore 命令补偿；若将来想把 majors 拆回独立 PR，需要先验证"同一 ecosystem+directory 拆两条条目 + 条目级 `exclude-patterns`"是否被 GitHub 侧校验接受（当前无公开先例）。
- **安全更新不参与分组与节奏**，需要随时单独处理。
- 本仓库不使用自动合并（`allow_auto_merge: false`，每次合并都需人工知情）。
