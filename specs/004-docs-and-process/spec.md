# 功能规格说明：文档与流程

**功能分支**：`004-docs-and-process`

**创建日期**：2026-07-08

**状态**：草稿

**输入**：参考 cc-switch 项目的文档体系（多语言指南、Release notes、用户手册、CI/CD 等）和 codex-plusplus 的工程化流程（ADRs、自动化依赖管理），为 MultiAgents Manager 建立完整的文档和开发流程规范。

## 用户场景与测试

### 用户故事 1 — 新用户快速上手（优先级: P1）

新用户下载应用后，需要清晰的文档了解功能和使用方法。当前仅有简短的 README.md，缺少详细的用户指南。

**优先级理由**：文档是用户留存的关键，没有文档的应用难以推广。

**独立测试**：新用户能在 10 分钟内根据文档完成首次使用。

**验收场景**：

1. **给定** 新用户首次打开应用，**当** 查看 `docs/user-manual/zh/` 文档，**则** 能在 10 分钟内理解会话看板、资源管理和预设组的基本用法
2. **给定** 用户遇到问题需要反馈，**当** 查看 `.github/ISSUE_TEMPLATE/`，**则** 能找到 bug 报告模板并正确填写
3. **给定** 用户想了解版本更新内容，**当** 查看 `docs/release-notes/` 或 CHANGELOG.md，**则** 能看到每个版本的变更摘要

### 用户故事 2 — 贡献者参与开发（优先级: P2）

外部开发者希望贡献代码，需要了解项目结构、开发流程和贡献规范。

**优先级理由**：开源项目需要清晰的贡献指南来降低参与门槛。

**独立测试**：新开发者能在 30 分钟内根据文档搭建开发环境并提交第一个 PR。

**验收场景**：

1. **给定** 新开发者克隆仓库，**当** 查看 `CONTRIBUTING.md`，**则** 能在 30 分钟内完成环境搭建、代码修改和 PR 提交
2. **给定** 开发者提交 PR，**当** CI 流水线运行，**则** 自动执行前端 lint + 后端 cargo check + 测试，失败时阻止合并
3. **给定** 开发者发现安全问题，**当** 查看 `SECURITY.md`，**则** 能按照流程安全地报告漏洞
4. **给定** 开发者提交 PR，**当** 按照 PR 模板填写，**则** 审查者能快速理解变更内容和影响范围

### 用户故事 3 — 多语言用户支持（优先级: P3）

项目已支持 i18n（中文/英文），但文档仅提供中文版本，需要补充英文文档以覆盖国际用户。

**优先级理由**：Tauri 应用天然支持跨平台，英文文档是国际化的基础。

**独立测试**：英文用户能根据英文文档完成应用安装和基本使用。

**验收场景**：

1. **给定** 英文用户访问文档，**当** 查看 `docs/user-manual/en/`，**则** 能看到与中文文档内容一致的英文版本
2. **给定** 英文用户查看 README，**当** 访问 `README.md`，**则** 能看到英文版本（或 README_EN.md）

### 用户故事 4 — 团队协作规范化（优先级: P4）

多人协作开发时，需要统一的代码审查流程、commit 规范、依赖管理策略。

**优先级理由**：商业化项目需要多人协作，没有规范会导致代码质量下降和合并冲突增多。

**独立测试**：新成员提交 PR 时自动触发 PR 模板、CODEOWNERS 分配审查者、commitlint 检查提交信息。

**验收场景**：

1. **给定** 开发者提交不符合 Conventional Commits 的 commit message，**当** 提交触发 commitlint，**则** 提交被拒绝并显示正确的格式示例
2. **给定** 开发者创建 PR 修改 `src-tauri/src/` 目录下的文件，**当** `CODEOWNERS` 配置生效，**则** 自动分配对应的代码审查者
3. **给定** Dependabot 检测到依赖有新版本，**当** 自动创建 PR，**则** CI 流水线运行并验证兼容性

## 功能需求

### FR-1: GitHub CI/CD

1. 创建 `.github/workflows/ci.yml` — 持续集成：
   ```yaml
   name: CI
   on:
     push:
       branches: [main, develop]
     pull_request:
       branches: [main]
   jobs:
     frontend:
       runs-on: ubuntu-latest
       steps:
         - uses: actions/checkout@v4
         - uses: pnpm/action-setup@v4
         - uses: actions/setup-node@v4
           with:
             node-version: 20
             cache: "pnpm"
         - run: pnpm install
         - run: pnpm lint
         - run: pnpm format:check
         - run: pnpm build
     backend:
       runs-on: ubuntu-latest
       steps:
         - uses: actions/checkout@v4
         - uses: dtolnay/rust-action@stable
         - run: cd src-tauri && cargo check
         - run: cd src-tauri && cargo clippy -- -D warnings
         - run: cd src-tauri && cargo test
   ```

2. 创建 `.github/workflows/release.yml` — 发布构建（5 平台）：
   ```yaml
   name: Release
   on:
     push:
       tags: ["v*"]
   jobs:
     build:
       strategy:
         matrix:
           platform: [macos-latest, ubuntu-latest, windows-latest]
       runs-on: ${{ matrix.platform }}
       steps:
         - uses: actions/checkout@v4
         - uses: pnpm/action-setup@v4
         - uses: actions/setup-node@v4
           with:
             node-version: 20
             cache: "pnpm"
         - run: pnpm install
         - run: pnpm tauri build
         - uses: tauri-apps/tauri-action@v0
           env:
             GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
   ```

3. 创建 `.github/workflows/stale.yml` — 自动标记 stale issue：
   ```yaml
   name: Close Stale Issues
   on:
     schedule:
       - cron: "0 0 * * *"
   jobs:
     stale:
       runs-on: ubuntu-latest
       steps:
         - uses: actions/stale@v9
           with:
             stale-issue-message: "此 issue 已闲置 60 天，如仍需处理请评论。"
             days-before-stale: 60
             days-before-close: 7
   ```

**cc-switch 参考**：`.github/workflows/ci.yml` + `release.yml` + `claude.yml` + `stale.yml` + `labeler.yml`

### FR-2: Issue 模板

1. 创建 `.github/ISSUE_TEMPLATE/bug_report.yml` — Bug 报告模板：
   ```yaml
   name: Bug 报告
   description: 报告一个 bug
   body:
     - type: textarea
       attributes:
         label: 问题描述
         description: 清晰描述 bug 现象
     - type: textarea
       attributes:
         label: 复现步骤
         description: 详细描述复现步骤
     - type: textarea
       attributes:
         label: 期望行为
         description: 描述期望的正确行为
     - type: textarea
       attributes:
         label: 环境信息
         description: 操作系统、应用版本等
   ```

2. 创建 `.github/ISSUE_TEMPLATE/feature_request.yml` — 功能请求模板：
   ```yaml
   name: 功能请求
   description: 建议新功能
   body:
     - type: textarea
       attributes:
         label: 功能描述
         description: 描述你希望的功能
     - type: textarea
       attributes:
         label: 使用场景
         description: 描述该功能的典型使用场景
     - type: textarea
       attributes:
         label: 替代方案
         description: 是否有替代方案
   ```

3. 创建 `.github/ISSUE_TEMPLATE/documentation.yml` — 文档改进模板
4. 创建 `.github/ISSUE_TEMPLATE/question.yml` — 问题咨询模板
5. 创建 `.github/ISSUE_TEMPLATE/config.yml` — 模板配置（禁用空白 issue）

**cc-switch 参考**：`.github/ISSUE_TEMPLATE/` 含 4 个 YAML 模板 + `config.yml`

### FR-3: 多语言指南

1. 创建 `docs/guides/` 目录结构：
   ```
   docs/guides/
   ├── en/
   │   ├── getting-started.md
   │   ├── session-monitoring.md
   │   ├── resource-management.md
   │   ├── preset-groups.md
   │   └── troubleshooting.md
   └── zh/
       ├── getting-started.md
       ├── session-monitoring.md
       ├── resource-management.md
       ├── preset-groups.md
       └── troubleshooting.md
   ```

2. 每个指南文件覆盖一个主题：
   - `getting-started.md` — 安装、首次启动、基本界面介绍
   - `session-monitoring.md` — 会话看板使用、状态含义、通知配置
   - `resource-management.md` — Skill/MCP/Plugin 的安装、启用、禁用
   - `preset-groups.md` — 预设组创建、应用、管理
   - `troubleshooting.md` — 常见问题排查

**cc-switch 参考**：`docs/guides/` 含 5 组三语指南（en/ja/zh）

### FR-4: Release notes

1. 创建 `docs/release-notes/` 目录结构：
   ```
   docs/release-notes/
   ├── v0.1.0.md
   ├── v0.2.0.md
   └── v0.3.0.md
   ```

2. 每个版本文档包含：
   - 版本号和发布日期
   - 新增功能列表
   - 改进项列表
   - 修复的 bug 列表
   - 已知问题
   - 升级注意事项

3. 采用 Keep a Changelog 格式

**cc-switch 参考**：`docs/release-notes/` 含每版本三语文档（v3.6.0 ~ v3.16.5）

### FR-5: 用户手册

1. 创建 `docs/user-manual/` 目录结构：
   ```
   docs/user-manual/
   ├── en/
   │   ├── 01-introduction.md
   │   ├── 02-installation.md
   │   ├── 03-dashboard.md
   │   ├── 04-sessions.md
   │   ├── 05-resources.md
   │   ├── 06-presets.md
   │   ├── 07-settings.md
   │   ├── 08-notifications.md
   │   ├── 09-shortcuts.md
   │   ├── 10-troubleshooting.md
   │   └── 11-faq.md
   └── zh/
       ├── 01-introduction.md
       ├── 02-installation.md
       ├── 03-dashboard.md
       ├── 04-sessions.md
       ├── 05-resources.md
       ├── 06-presets.md
       ├── 07-settings.md
       ├── 08-notifications.md
       ├── 09-shortcuts.md
       ├── 10-troubleshooting.md
       └── 11-faq.md
   ```

2. 每章内容约 500-1000 字，含截图占位符

**cc-switch 参考**：`docs/user-manual/{en,ja,zh}/` 分章节手册（31 页/语言）

### FR-6: CHANGELOG.md

1. 创建根目录 `CHANGELOG.md`，采用 Keep a Changelog 格式：
   ```markdown
   # Changelog

   ## [Unreleased]

   ## [0.3.0] - 2026-07-08
   ### Added
   - 资源管理看板双视图（按类型/按工具）
   - 预设组一键应用/取消功能
   - 兼容性检查对话框

   ### Fixed
   - 修复 TypeScript 未使用变量错误

   ## [0.2.0] - 2026-07-05
   ### Added
   - 多 Agent 工具统一监控
   - Skill/MCP/Plugin 三层映射架构
   - 系统托盘预设菜单

   ## [0.1.0] - 2026-07-01
   ### Added
   - 初始版本：会话监控看板、状态通知、终端跳转
   ```

**cc-switch 参考**：根目录 `CHANGELOG.md`

### FR-7: CONTRIBUTING.md

1. 创建根目录 `CONTRIBUTING.md`，包含：
   - 项目技术栈说明
   - 开发环境搭建步骤
   - 代码风格规范（Rust fmt + Prettier）
   - 提交信息规范（Conventional Commits）
   - PR 流程（fork → branch → commit → push → PR）
   - 审查标准（测试通过、文档更新、无回归）
   - 本地测试命令（`cargo test`、`pnpm test`）

**cc-switch 参考**：根目录 `CONTRIBUTING.md`

### FR-8: SECURITY.md

1. 创建根目录 `SECURITY.md`，包含：
   - 安全策略概述
   - 支持的版本列表
   - 漏洞报告流程（邮箱或 GitHub Security Advisories）
   - 漏洞披露时间线
   - 安全更新通知方式

**cc-switch 参考**：根目录 `SECURITY.md`

### FR-9: Screenshots

1. 将 `TEST/` 目录中的临时截图迁移到 `assets/screenshots/`（现阶段暂不执行迁移，见 FR-9.3；待后续阶段 UI 稳定后再迁移）
2. 创建 `assets/screenshots/` 目录结构：
   ```
   assets/screenshots/
   ├── zh/
   │   ├── dashboard.png
   │   ├── resources.png
   │   ├── presets.png
   │   └── settings.png
   └── en/
       ├── dashboard.png
       ├── resources.png
       ├── presets.png
       └── settings.png
   ```

3. `TEST/` 目录维持 `.gitignore` 忽略状态，不删除条目，不做处理
4. 在 README.md 和文档中引用 `assets/screenshots/` 中的图片

**cc-switch 参考**：`assets/screenshots/` 多语言应用截图

### FR-10: Architecture Decision Records (ADRs)

1. 创建 `docs/adr/` 目录，采用轻量级 ADR 格式（参考 codex-plusplus 的 `docs/ARCHITECTURE.md`）：
   ```
   docs/adr/
   ├── README.md          # ADR 索引
   ├── 001-adapter-pattern.md
   ├── 002-three-layer-symlink.md
   ├── 003-hook-priority-status.md
   └── 004-react-query-vs-polling.md
   ```

2. 每个 ADR 包含：
   - **状态**：提议 / 已接受 / 已废弃 / 已取代
   - **背景**：为什么需要做这个决策
   - **决策**：选择了什么方案
   - **后果**：选择带来的正面和负面影响
   - **替代方案**：考虑过但未采纳的方案及原因

3. ADR 模板示例：
   ```markdown
   # ADR 001: Adapter 模式实现多工具支持

   **状态**：已接受
   **日期**：2026-07-05

   ## 背景
   需要支持多个 AI 编程工具（Claude Code、Codex CLI、OpenCode 等），
   每个工具的进程发现、配置格式、会话解析方式不同。

   ## 决策
   采用 Adapter 模式：每个工具实现 `AgentAdapter` trait，核心模块
   通过 trait 接口与工具交互，不感知具体工具实现。

   ## 后果
   - 正面：新增工具只需实现 trait，不改核心代码
   - 负面：trait 设计需足够抽象，可能限制工具特有功能的利用

   ## 替代方案
   - 硬编码 if-else：代码膨胀快，拒绝
   - 插件系统：过于复杂，MVP 阶段过度设计，拒绝
   ```

4. 将现有设计决策（来自 CLAUDE.md、项目宪法）逐步迁移到 ADR 格式

**codex-plusplus 参考**：`docs/ARCHITECTURE.md` — 详细的架构决策文档，包含"为什么这样选"的理由

### FR-11: Conventional Commits 强制执行

> **归属说明**：commitlint/`commit-msg` hook 的完整配置归本 spec（004）单一归属；Spec 002 的 FR-11 只负责 `pre-commit` 的 lint-staged，不在 `.husky/commit-msg` 上重复配置。

1. 安装依赖：`pnpm add -D @commitlint/cli @commitlint/config-conventional`
2. 创建 `commitlint.config.js`：
   ```javascript
   export default {
     extends: ["@commitlint/config-conventional"],
     rules: {
       "type-enum": [2, "always", [
         "feat", "fix", "docs", "style", "refactor",
         "perf", "test", "chore", "ci", "build", "revert"
       ]],
       "scope-case": [2, "always", "lower-case"],
     },
   };
   ```
3. 创建 `.husky/commit-msg`：
   ```bash
   npx --no -- commitlint --edit $1
   ```
4. 在 `CONTRIBUTING.md` 中说明 commit 规范：
   - `feat:` 新功能
   - `fix:` Bug 修复
   - `docs:` 文档更新
   - `refactor:` 代码重构
   - `test:` 测试相关
   - `chore:` 构建/工具链

**cc-switch 参考**：commitlint + husky 强制执行 Conventional Commits

> **建议**：`pre-commit` 仅跑 lint-staged（变更文件 format + lint），`pnpm build` 交给 CI，避免提交期跑全量 build 拖慢。

### FR-12: 自动化依赖更新

1. 创建 `.github/dependabot.yml`：
   ```yaml
   version: 2
   updates:
     - package-ecosystem: "npm"
       directory: "/"
       schedule:
         interval: "weekly"
         day: "monday"
       open-pull-requests-limit: 10
       labels:
         - "dependencies"
         - "frontend"
       groups:
         minor-patch:
           update-types:
             - "minor"
             - "patch"
     - package-ecosystem: "cargo"
       directory: "/src-tauri/"
       schedule:
         interval: "weekly"
         day: "monday"
       open-pull-requests-limit: 10
       labels:
         - "dependencies"
         - "backend"
   ```

2. Dependabot 配置要点：
   - 前端（npm）+ 后端（cargo）分别配置
   - 每周一自动检查
   - 按 minor/patch 分组创建 PR，减少 PR 噪音（npm groups）
   - 自动打标签 `dependencies` + `frontend`/`backend`
   - PR 自动触发 CI 验证兼容性

**cc-switch 参考**：Dependabot 自动化依赖更新

### FR-13: CODEOWNERS 文件

1. 创建 `.github/CODEOWNERS`：
   ```
   # 后端 Rust 代码
   /src-tauri/  @backend-maintainers

   # 前端 React 代码
   /src/        @frontend-maintainers

   # 文档
   /docs/       @docs-maintainers
   /specs/      @docs-maintainers

   # CI/CD 配置
   /.github/    @devops-maintainers

   # 项目配置
   /package.json @frontend-maintainers @backend-maintainers
   ```

2. CODEOWNERS 作用：
   - PR 自动分配审查者
   - 关键文件修改需要特定团队批准
   - 保护核心模块不被无意修改

**cc-switch 参考**：CODEOWNERS 文件，多人协作时自动分配审查者

### FR-14: PR 模板

1. 创建 `.github/PULL_REQUEST_TEMPLATE.md`：
   ```markdown
   ## 变更描述

   <!-- 简要描述此 PR 做了什么 -->

   ## 变更类型

   - [ ] 新功能（feat）
   - [ ] Bug 修复（fix）
   - [ ] 文档更新（docs）
   - [ ] 代码重构（refactor）
   - [ ] 测试（test）
   - [ ] 工程化（chore）

   ## 测试

   - [ ] `cargo test` 通过
   - [ ] `pnpm test` 通过
   - [ ] `pnpm check` 通过
   - [ ] 手动验证通过（附截图/描述）

   ## 检查清单

   - [ ] 代码已自审
   - [ ] 相关文档已更新
   - [ ] CHANGELOG 已更新（如有用户可见变更）
   - [ ] 无安全风险（敏感路径、凭据泄露等）

   ## 截图（如涉及 UI 变更）

   <!-- 粘贴截图 -->
   ```

2. PR 模板强制开发者填写关键信息，减少审查者来回沟通成本

### FR-15: 开发环境标准化（可选）

1. 创建 `.devcontainer/devcontainer.json`（可选，为 VS Code / GitHub Codespaces 用户预配置开发环境）：
   ```json
   {
     "name": "MultiAgents Manager",
     "image": "mcr.microsoft.com/devcontainers/rust:1",
     "features": {
       "ghcr.io/devcontainers/features/node:1": { "version": "20" }
     },
     "postCreateCommand": "pnpm install && cd src-tauri && cargo build",
     "customizations": {
       "vscode": {
         "extensions": [
           "rust-lang.rust-analyzer",
           "bradlc.vscode-tailwindcss",
           "esbenp.prettier-vscode"
         ]
       }
     }
   }
   ```

2. 此项为可选，降低新开发者的环境搭建门槛

## 成功标准

1. `.github/workflows/` 中 CI 流水线能成功运行（前端 lint + 后端 cargo check + test）
2. `.github/ISSUE_TEMPLATE/` 中 4 个 issue 模板能在 GitHub 上正常显示
3. `docs/guides/` 中每个指南文件 >=500 字，覆盖 5 个主题
4. `docs/user-manual/` 中每个语言版本 >=10 章，每章 >=500 字
5. `CHANGELOG.md` 包含所有已发布版本的变更记录
6. `CONTRIBUTING.md` 包含完整的环境搭建和贡献流程说明
7. `SECURITY.md` 包含漏洞报告流程和披露时间线
8. `assets/screenshots/` 包含 >=4 张应用截图（中英文各一套）
9. `docs/adr/` 包含 >=4 个关键架构决策记录
10. commitlint 阻止不符合 Conventional Commits 的提交
11. Dependabot 每周自动检查依赖更新并创建 PR
12. CODEOWNERS 在 PR 创建时自动分配审查者

## 关键实体

| 实体 | 说明 | 关键属性 |
|------|------|---------|
| Workflow | GitHub Actions 工作流 | 触发条件、任务步骤、运行环境 |
| IssueTemplate | Issue 模板 | 类型、字段、默认值 |
| Guide | 用户指南 | 主题、语言、内容、截图 |
| ReleaseNote | 版本发布说明 | 版本号、日期、新增/改进/修复 |
| Screenshot | 应用截图 | 页面、语言、分辨率 |
| ADR | 架构决策记录 | 状态、背景、决策、后果、替代方案 |
| CODEOWNERS | 代码所有权映射 | 路径模式、团队/用户 |

## 假设

1. 文档在功能稳定后编写（Spec 001、002、003 完成后）；FR-1 的 CI 跑 `cargo test` + `pnpm test`，依赖 Spec 003 的测试基础设施。
2. GitHub 仓库已公开，CI/CD 和 issue 模板能正常工作
3. 截图在应用 UI 稳定后生成，使用高分辨率显示器
4. 英文文档由中文文档翻译而来，保持内容一致
5. CHANGELOG 从当前版本开始维护，历史版本简要概括
6. ADR 从当前开始记录，历史决策追溯编写
7. Dependabot 在仓库公开后方可启用
8. CODEOWNERS 中的团队名称需要先在 GitHub 组织/仓库中创建

## 风险评估

| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| 文档与代码不同步 | 中 | 在 PR 模板中增加"文档是否更新"检查项 |
| 多语言文档维护成本高 | 中 | 优先维护中文文档，英文文档后续补充 |
| CI 流水线配置错误 | 低 | 先在个人 fork 上测试，确认无误后再合并 |
| 截图过时 | 低 | 每次 UI 变更时同步更新截图，或标注版本号 |
| Issue 模板字段过多导致用户放弃 | 低 | 保持模板简洁，必填字段控制在 3 个以内 |
| commitlint 过于严格导致开发者抵触 | 低 | 提供清晰的 commit 范例和 CONTRIBUTING 文档 |
| Dependabot PR 过多导致噪音 | 低 | 配置 `open-pull-requests-limit: 10` + minor/patch 分组 |

## cc-switch 参考

- **GitHub CI/CD**：`.github/workflows/ci.yml`(前端+后端并行检查) + `release.yml`(5平台构建) + `stale.yml`
- **Issue 模板**：`.github/ISSUE_TEMPLATE/` 含 4 个 YAML 模板 + `config.yml`
- **多语言指南**：`docs/guides/` 含 5 组三语指南（en/ja/zh）
- **Release notes**：`docs/release-notes/` 含每版本三语文档
- **用户手册**：`docs/user-manual/{en,ja,zh}/` 分章节手册（31页/语言）
- **CHANGELOG**：根目录 `CHANGELOG.md`
- **CONTRIBUTING**：根目录 `CONTRIBUTING.md`
- **SECURITY**：根目录 `SECURITY.md`
- **Screenshots**：`assets/screenshots/` 多语言应用截图

## codex-plusplus 参考

- **ADR 模式**：`docs/ARCHITECTURE.md` — 详细记录设计决策和"为什么这样选"
- **Monorepo 工程化**：`tsconfig.base.json`、`packages/` 结构
- **CLI 命令规范**：`packages/installer/src/commands/` 按功能拆分的命令文件
- **Contributing 规范**：`CONTRIBUTING.md` 含开发命令、发布检查清单
- **Security 策略**：`SECURITY.md` 含漏洞报告和披露流程
- **CI 配置**：`.github/workflows/ci.yml` 单个 macOS 构建验证
