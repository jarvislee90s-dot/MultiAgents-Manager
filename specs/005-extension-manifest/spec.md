# 功能规格说明：Extension Manifest 标准化

**功能分支**：`005-extension-manifest`

**创建日期**：2026-07-08

**状态**：草稿

**输入**：参考 codex-plusplus（3473 stars）的 tweak manifest 系统和 cc-switch 的扩展管理模式，为 MultiAgents Manager 的 Skill / MCP / Plugin 三类资源定义统一的 manifest 规范，支撑资源发现、验证、安装、权限控制和版本管理。

## 用户场景与测试

### 用户故事 1 — 用户安全安装资源（优先级: P1）

用户从 GitHub 下载了一个 MCP 服务器，想安装到 MAM。安装前需要了解这个 MCP 需要什么权限、兼容哪些工具、支持什么配置格式。当前没有统一的元数据格式，用户只能靠 README 了解信息，容易遗漏安全风险。

**优先级理由**：安全透明是项目宪法原则 VII 的强制要求——"每个 skill/MCP/插件的安装路径和权限必须对用户可见"。

**独立测试**：安装一个携带 `mam.json` 的 MCP 资源，验证安装前 UI 正确展示权限列表和兼容性信息。

**验收场景**：

1. **给定** 用户导入一个携带 `mam.json` 的 MCP 资源，**当** 安装界面展示，**则** 可以看到资源名称、版本、权限声明（network、filesystem.read）、兼容的工具列表
2. **给定** manifest 声明了 `shell` 权限，**当** 用户查看权限列表，**则** `shell` 权限标记为高风险（红色警告），并显示说明"可执行任意 shell 命令"
3. **给定** manifest 中 `compatibility` 不包含用户当前使用的工具，**当** 用户尝试为该工具启用该资源，**则** 系统显示兼容性警告并提供"仍然启用（不推荐）"选项
4. **给定** manifest 缺失必填字段（如 `id` 或 `name`），**当** 用户尝试安装，**则** 安装被拒绝并显示具体的校验错误信息

### 用户故事 2 — 资源开发者发布资源（优先级: P2）

开发者创建了一个 skill，希望发布到 MAM 资源商店供其他用户安装。需要按照规范编写 manifest，通过校验后才能发布。

**优先级理由**：标准化 manifest 是资源生态的基础，没有统一规范就无法建立资源商店。

**独立测试**：开发者使用 CLI 命令校验自写的 `mam.json`，验证通过后资源出现在本地商店索引中。

**验收场景**：

1. **给定** 开发者编写了 `mam.json`，**当** 运行 `mam validate ./my-skill`，**则** 校验通过显示"验证成功"并列出资源摘要（类型、版本、权限），校验失败则显示错误位置和原因
2. **给定** 开发者的资源已通过校验，**当** 运行 `mam publish ./my-skill`，**则** 资源被添加到 `~/.mam/store/index.json`（本地商店），应用 UI 的"可用资源"列表中显示该资源
3. **给定** 开发者更新了资源的版本号，**当** 旧版本用户查看已安装资源列表，**则** 显示"有新版本可用"并提供更新入口

### 用户故事 3 — 系统校验与安全防护（优先级: P3）

系统需要在安装资源时对 manifest 进行严格校验，并在运行时根据权限声明控制资源行为。

**优先级理由**：第二阶段的目标——从"声明式展示"升级到"运行时强制"，防止恶意资源越权操作。

**独立测试**：安装一个声明 `permissions: ["filesystem.read"]` 的 MCP，当它尝试写入文件时，系统拦截并报错。

**验收场景**：

1. **给定** MCP 声明了 `filesystem.read` 但尝试 shell 命令，**当** 运行时检测到越权调用，**则** 操作被拦截，用户收到通知"MCP xxx 尝试执行 shell 命令但未声明 shell 权限"
2. **给定** Skill 未声明任何权限，**当** 其 SKILL.md 指令尝试让 Agent 读取 `.env` 文件，**则** 敏感路径保护生效（宪法 VII），操作被拒绝

## 功能需求

### FR-1: Manifest Schema 定义

1. 定义 `mam.json` 作为所有资源类型的统一 manifest 格式，字段规范如下：

**公共字段（所有类型）**：

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `id` | `string` | 是 | 全局唯一标识，reverse-DNS 格式。允许字符：字母、数字、`.`、`_`、`-`。如 `com.example.brainstorming` |
| `name` | `string` | 是 | 人可读的展示名称 |
| `version` | `string` | 是 | semver，如 `1.2.0` |
| `kind` | `"skill" \| "mcp" \| "plugin"` | 是 | 资源类型 |
| `description` | `string` | 否 | 一行描述，展示在资源名称下方 |
| `author` | `string \| { name, url?, email? }` | 否 | 作者信息 |
| `homepage` | `string`（URL） | 否 | 项目主页 |
| `iconUrl` | `string` | 否 | 图标，支持 `https://`、`data:`、相对路径。相对路径从资源目录读取，限制 1 MiB |
| `tags` | `string[]` | 否 | 搜索/分类标签，如 `["ui", "shortcut", "filesystem"]` |
| `minRuntime` | `string` | 否 | 最低 MAM 版本要求，如 `>=0.3.0` |
| `githubRepo` | `string`（`owner/repo`） | 否 | 用于版本更新检查 |
| `permissions` | `Permission[]` | 否 | 权限声明列表（见 FR-2） |
| `compatibility` | `CompatibilityEntry[]` | 否 | 兼容的 Agent 工具列表（见 FR-3） |

**Skill 扩展字段（`kind: "skill"`）**：

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `skill.entry` | `string` | 是 | SKILL.md 入口文件路径，相对于资源根目录 |
| `skill.includes` | `string[]` | 否 | 额外需要链接的文件/目录，支持 glob 模式 |

**MCP 扩展字段（`kind: "mcp"`）**：

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `mcp.command` | `string` | 是 | 启动命令 |
| `mcp.args` | `string[]` | 否 | 命令参数 |
| `mcp.env` | `Record<string, string>` | 否 | 环境变量 |
| `mcp.formats` | `("json" \| "toml" \| "jsonc")[]` | 否 | 支持的配置格式，默认 `["json"]`。未列出的格式在对应工具上不可用 |

**Plugin 扩展字段（`kind: "plugin"`）**：

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `plugin.entry` | `string` | 是 | 入口文件路径 |
| `plugin.type` | `"file" \| "config" \| "mixed"` | 是 | 插件类型：file（符号链接部署）、config（配置写入）、mixed（两者都有） |
| `plugin.configTemplate` | `string` | 否 | 配置模板路径（type 为 config/mixed 时） |

2. 示例 — Skill manifest：

```json
{
  "id": "com.example.brainstorming",
  "name": "Brainstorming",
  "version": "1.0.0",
  "kind": "skill",
  "description": "在任何创造性工作之前进行头脑风暴，探索用户意图和设计",
  "author": { "name": "Example Org", "url": "https://example.com" },
  "homepage": "https://github.com/example/brainstorming",
  "iconUrl": "./icon.png",
  "tags": ["creative", "planning"],
  "minRuntime": "0.3.0",
  "githubRepo": "example/brainstorming",
  "permissions": ["filesystem.read"],
  "compatibility": [
    { "tool": "claude", "minVersion": "1.0.0" },
    { "tool": "codex", "minVersion": "0.5.0" },
    { "tool": "opencode" }
  ],
  "skill": {
    "entry": "SKILL.md",
    "includes": ["references/", "scripts/"]
  }
}
```

3. 示例 — MCP manifest：

```json
{
  "id": "com.example.filesystem-mcp",
  "name": "Filesystem MCP Server",
  "version": "1.0.0",
  "kind": "mcp",
  "description": "安全的文件系统访问 MCP 服务器",
  "author": { "name": "ModelContextProtocol", "url": "https://modelcontextprotocol.io" },
  "homepage": "https://github.com/modelcontextprotocol/servers",
  "iconUrl": "https://raw.githubusercontent.com/.../icon.png",
  "tags": ["filesystem", "mcp", "utility"],
  "minRuntime": "0.3.0",
  "githubRepo": "modelcontextprotocol/servers",
  "permissions": ["filesystem.read", "filesystem.write"],
  "compatibility": [
    { "tool": "claude", "mcpFormat": "json" },
    { "tool": "codex", "mcpFormat": "toml" },
    { "tool": "opencode", "mcpFormat": "jsonc" }
  ],
  "mcp": {
    "command": "npx",
    "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/allowed"],
    "env": {},
    "formats": ["json", "toml", "jsonc"]
  }
}
```

4. 示例 — Plugin manifest：

```json
{
  "id": "com.example.notification-plugin",
  "name": "Enhanced Notifications",
  "version": "1.0.0",
  "kind": "plugin",
  "description": "增强系统通知，支持自定义模板和条件触发",
  "author": { "name": "MAM Team" },
  "tags": ["notification", "ui"],
  "minRuntime": "0.3.0",
  "permissions": ["settings.write", "shell"],
  "compatibility": [
    { "tool": "claude", "minVersion": "1.0.0" }
  ],
  "plugin": {
    "entry": "dist/notification-plugin.js",
    "type": "mixed",
    "configTemplate": "config.template.json"
  }
}
```

**cc-switch 参考**：无直接对应（cc-switch 无统一 manifest 系统）

**codex-plusplus 参考**：`tweaks/<id>/manifest.json` — 统一 JSON manifest 含 id/name/version/githubRepo/permissions/scope/mcp 字段

### FR-2: 权限模型

1. 定义权限枚举（Permission）：

| 权限 | 含义 | 风险等级 | 适用类型 |
|------|------|---------|---------|
| `filesystem.read` | 读取文件 | 低 | Skill / MCP / Plugin |
| `filesystem.write` | 写入文件 | 中 | Skill / MCP / Plugin |
| `network` | 发起网络请求 | 中 | MCP / Plugin |
| `shell` | 执行 shell 命令 | 高 | MCP / Plugin |
| `env.read` | 读取环境变量 | 中 | MCP / Plugin |
| `settings.write` | 写入工具配置文件 | 高 | MCP / Plugin |
| `symlink.create` | 创建符号链接 | 低 | Skill |

2. 风险等级对应的 UI 行为：
   - 低（绿色）：正常展示，无需特殊警告
   - 中（黄色）：安装时展示说明文字
   - 高（红色）：安装时弹出强制确认对话框，显示具体风险

3. 分阶段实施计划：
   - **Phase 1（当前）**：manifest 声明权限，UI 在安装前展示 + 确认对话框。Schema 支持完整权限枚举。不做运行时强制
   - **Phase 2（后续）**：前端 API 层（`src/lib/api/`）在 invoke 调用前检查权限 token，越权操作拒绝并在 UI 展示。后端 service 层同样检查

**codex-plusplus 参考**：`manifest.json#permissions` — 12 种权限（settings/ipc/filesystem/network/codex-runtime/codex-windows/codex-views/codex-cdp/native-module/native-view/native-helper），运行时 enforce 原生桥接权限

### FR-3: 兼容性声明

1. `compatibility` 数组条目字段：

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `tool` | `string` | 是 | Agent 工具标识，如 `claude`、`codex`、`opencode`、`openclaw` |
| `minVersion` | `string` | 否 | 该工具的最低版本要求，如 `1.0.0` |
| `mcpFormat` | `string` | 否 | MCP 配置格式（仅 MCP 资源），如 `json`、`toml`、`jsonc` |
| `subAgentSupport` | `boolean` | 否 | 是否支持子 Agent（仅 Skill），默认 `false` |
| `notes` | `string` | 否 | 额外兼容性说明 |

2. 兼容性检查逻辑：
   - 安装资源时检查目标工具是否在 `compatibility` 列表中
   - 如果不在，显示"未经测试"警告，允许用户强行安装
   - 如果工具的版本低于 `minVersion`，显示版本不满足警告

3. MCP 格式适配：
   - 如果 `mcp.formats` 不包含工具的格式，该工具不在可选列表中
   - 例如：MCP 只支持 `json` 格式 → 只能在 Claude Code（JSON）上启用，不能在 Codex CLI（TOML）上启用

### FR-4: Manifest 校验工具

1. 创建 `src-tauri/src/services/manifest/validator.rs` — Rust 侧 manifest 校验器：
   ```rust
   pub struct ManifestValidator;

   impl ManifestValidator {
       pub fn validate_file(path: &Path) -> Result<Manifest, Vec<ValidationError>>;
       pub fn validate_json(json: &str) -> Result<Manifest, Vec<ValidationError>>;
   }
   ```

2. 校验规则：
   - 必填字段检查（`id`、`name`、`version`、`kind`）
   - 字段类型检查（`version` 为有效 semver，`kind` 为有效枚举值）
   - 字段格式检查（`id` 仅含合法字符，`githubRepo` 格式为 `owner/repo`）
   - 权限有效性检查（权限值在枚举范围内）
   - 类型必填字段检查（Skill 需要 `skill.entry`，MCP 需要 `mcp.command`，Plugin 需要 `plugin.entry` + `plugin.type`）
   - 相对路径安全检查（不允许 `../` 穿越目录）

3. 命令行校验工具（Phase 2，独立的 Rust CLI binary）：
   ```bash
   mam validate ./my-skill     # 校验单个资源
   mam validate --all           # 校验所有已安装资源
   mam validate --json          # JSON 格式输出结果
   ```

4. 创建 `src/lib/schemas/manifest.ts` — 前端 Zod schema：
   ```typescript
   import { z } from "zod";

   export const PermissionSchema = z.enum([
     "filesystem.read", "filesystem.write",
     "network", "shell", "env.read",
     "settings.write", "symlink.create",
   ]);

   export const CompatibilityEntrySchema = z.object({
     tool: z.string(),
     minVersion: z.string().optional(),
     mcpFormat: z.enum(["json", "toml", "jsonc"]).optional(),
     subAgentSupport: z.boolean().optional(),
     notes: z.string().optional(),
   });

   export const ManifestSchema = z.discriminatedUnion("kind", [
     z.object({
       kind: z.literal("skill"),
       id: z.string().regex(/^[a-zA-Z0-9._-]+$/),
       name: z.string().min(1),
       version: z.string().regex(/^\d+\.\d+\.\d+/),
       // ... 公共字段
       skill: z.object({
         entry: z.string(),
         includes: z.array(z.string()).optional(),
       }),
     }),
     // ... mcp / plugin 变体
   ]);
   ```

**codex-plusplus 参考**：CLI `validate-tweak` 命令 + SDK `validateTweakManifest()` 函数

### FR-5: Store 索引格式

1. 创建 `~/.mam/store/` 目录结构（参考 codex-plusplus 的 `store/`）：
   ```
   ~/.mam/store/
   ├── index.json          # 资源商店索引
   └── icons/              # 资源图标缓存
       ├── com.example.brainstorming.png
       └── com.example.filesystem-mcp.png
   ```

2. `index.json` 格式：
   ```json
   {
     "version": "1",
     "updated": "2026-07-08T10:00:00Z",
     "entries": [
       {
         "id": "com.example.brainstorming",
         "name": "Brainstorming",
         "kind": "skill",
         "version": "1.0.0",
         "description": "在任何创造性工作之前进行头脑风暴",
         "author": { "name": "Example Org" },
         "tags": ["creative", "planning"],
         "iconUrl": "icons/com.example.brainstorming.png",
         "githubRepo": "example/brainstorming",
         "downloadUrl": "https://github.com/example/brainstorming/releases/latest/download/skill.tar.gz",
         "sha256": "abc123...",
         "installed": 0,
         "featured": false
       }
     ]
   }
   ```

3. 本地商店（MVP）vs 远程商店（Phase 2）：
   - MVP：`index.json` 存储在本地，通过 `mam publish` 或 UI 导入添加
   - Phase 2：支持远程 `index.json` URL，定期同步，类似 codex-plusplus 的 GitHub Releases 检查

**codex-plusplus 参考**：`store/index.json` + `store/icons/` — 中心化 tweak 商店索引

### FR-6: 安装与分发流程

1. 本地安装流程（从目录/GitHub 导入）：
   ```
   用户选择资源目录
     → 系统读取 mam.json
     → 校验 manifest（FR-4）
     → 展示资源信息 + 权限列表（FR-2）
     → 用户确认安装
     → 资源文件复制到 ~/.mam/skills/<id>/（或 ~/.mam/mcp/<id>/）
     → 写入 store/index.json
     → 数据库记录 extension 实体
   ```

2. 商店安装流程（从 store 索引安装）：
   ```
   用户浏览可用资源列表
     → 点击安装
     → 下载资源包（验证 sha256）
     → 后续同本地安装流程
   ```

3. 更新检查流程：
   ```
   定时（每天一次）检查已安装资源的 githubRepo
     → 对比本地 manifest.version 和 GitHub Release tag
     → 有更新时在 UI 显示"更新可用"
     → 用户手动触发更新（不自动安装）
   ```

4. 卸载流程：
   ```
   用户卸载资源
     → 移除文件（~/.mam/skills/<id>/）
     → 移除所有工具的符号链接/配置
     → 数据库标记为未安装
     → store/index.json 保留条目（installCount 减 1）
   ```

### FR-7: 与现有系统的集成

1. 对接 Spec 001 的扩展资源实体 — `extension` 表新增字段：
   - `manifest_path` — manifest 文件路径
   - `permissions` — 权限列表（逗号分隔）
   - `min_runtime` — 最低 MAM 版本

2. 对接 Spec 002 的代码架构（显式 FR 引用）：
   - 后端校验器放在 `src-tauri/src/services/manifest/` -> 依赖 002 FR-5（services/ 重命名拆分）
   - 前端 Zod schema 放在 `src/lib/schemas/manifest.ts` -> 依赖 002 FR-7（zod 安装 + schemas/ 创建）
   - 前端 API 层新增 `src/lib/api/manifest.ts` -> 依赖 002 FR-4（api 层）
   - UI 组件放在 `src/components/resources/` -> 依赖 002 FR-3（components 子目录化）

3. 对接 Spec 001 的三层映射 - 安装到 Layer 1（SSOT，`~/.mam/skills/<id>/`），manifest 中的 `compatibility` 决定可在哪些工具上启用。Skill 通过 Layer 2（`~/.mam/active/<tool>/`）符号链接映射；MCP 不走 Layer，直接写入工具配置文件，安装时在 `~/.mam/mcp/<id>/` 记录元数据。

4. MCP 格式适配 — manifest 的 `mcp.formats` 字段决定该 MCP 能以什么格式写入各工具的配置：
   - Claude Code → `json` → `~/.claude.json#mcpServers`
   - Codex CLI → `toml` → `~/.codex/config.toml#[mcp_servers]`
   - OpenCode → `jsonc` → `opencode.json#mcp`

## 成功标准

1. Manifest schema 完整定义，覆盖所有三类资源（Skill / MCP / Plugin）
2. 安装流程触发 ManifestValidator 校验，能正确拒绝缺失必填字段/非法 semver/不合法 `githubRepo` 格式/含 `../` 路径穿越的 manifest，并返回结构化 ValidationError 列表（含字段路径 + 错误信息 + 错误码），通过手动安装流程可在 Phase 1 验证
3. 安装资源时 UI 正确展示权限列表和风险等级（红/黄/绿）
4. 兼容性检查能阻止在不兼容的工具上安装资源（并给出明确提示）
5. `store/index.json` 格式稳定，支持本地和远程商店
6. 版本更新检查功能可用，UI 正确显示"有新版本"
7. 所有现有已安装资源（无 manifest）不受影响，标记为"legacy"（无 manifest 模式）

## 关键实体

| 实体 | 说明 | 关键属性 |
|------|------|---------|
| Manifest | 资源的自描述元数据 | id、name、version、kind、permissions、compatibility |
| Permission | 权限声明 | 名称、风险等级、适用类型 |
| CompatibilityEntry | 兼容性声明 | tool、minVersion、mcpFormat、subAgentSupport |
| StoreEntry | 商店索引条目 | id、版本、下载 URL、sha256、安装次数 |
| ValidationError | 校验错误 | 字段路径、错误信息、错误码 |

## 假设

1. Manifest 遵循 semver 版本规范
2. 资源 ID 遵循 reverse-DNS 格式，全局唯一
3. 现有无 manifest 的资源（legacy）继续正常工作，不被强制要求迁移
4. Phase 1 只做声明式权限展示，不做运行时强制
5. 远程商店（Phase 2）需要额外的服务端支持，不在 Phase 1 范围
6. GitHub Release 格式为标准 semver tag（`v1.0.0` 或 `1.0.0`）
7. 本 spec 依赖 002 的 FR-3/FR-4/FR-5/FR-7 完成后才可落地。

## 风险评估

| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| Legacy 资源（无 manifest）与新系统不兼容 | 中 | 标记为 legacy，UI 中提示"无 manifest，无法验证安全性" |
| 权限枚举不够用（未来新增资源类型需要新权限） | 低 | 权限枚举设计为可扩展，新增权限不影响现有资源 |
| Store 索引被篡改或投毒 | 高 | sha256 校验 + Phase 2 引入签名验证 |
| Manifest schema 过于复杂，阻碍第三方开发者 | 中 | 必填字段精简到 4 个（id/name/version/kind），其余全可选 |

## cc-switch 参考

无直接对应的 manifest 系统。cc-switch 的资源管理通过数据库 schema 和 DAO 层实现，未使用独立的 manifest 文件。

## codex-plusplus 参考

- **Manifest schema**：`docs/tweaks/manifest.md` — 完整的 manifest 规范，含 required/optional 字段表 + 验证规则
- **权限系统**：`manifest.json#permissions` — 12 种声明式权限，原生桥接权限运行时 enforce
- **MCP 声明**：`manifest.json#mcp` — 在 manifest 中直接声明 MCP server（command/args/env）
- **Store 格式**：`store/index.json` — 中心化 tweak 商店索引
- **CLI 校验**：`codexplusplus validate-tweak` 命令
- **SDK 校验**：`packages/sdk` 提供 `validateTweakManifest()` 函数
- **版本检查**：通过 `githubRepo` 检查 GitHub Releases，每天一次，不自动安装
