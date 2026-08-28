# 功能规格说明：资源管理功能修复（P0 数据安全 + P1 功能缺陷 + P2 工程质量）

**功能分支**：`015-resource-management-fixes`

**创建日期**：2026-08-25

**状态**：草稿

**输入**：与 cc-switch（main 分支）对比审查发现的缺陷清单。P0-① Windows 上禁用原生 skill 会永久删除（`trash` 外部命令不存在时回退 `remove_link`，与 UI"移至回收站"文案不符）；P1 共 5 项功能缺陷（`uninstall_resource` 参数与路径错误、JSONC 注释丢失、扫描去重误伤、新装 skill 不被发现、断链不检测）；P2 共 3 项工程质量项（死代码清理与接线、扫描/安装加固、批量操作与搜索）。
P0-②（读取失败不得以空对象覆写）与 P0-③（写前无备份）属于写路径基础设施，由 `016-config-robustness-hardening` 承接实现，本 spec 不重复展开。

## 用户场景与测试

### 用户故事 1 — 禁用原生 skill 进入系统回收站（优先级: P0）

**验收场景**：

1. **给定** Windows 上某工具 skill 目录下的非链接原生 skill（`checkSkillTargetType` 返回 `native`），**当** 用户确认禁用，**则** 文件进入系统回收站（资源管理器可恢复），DB 记录禁用状态，返回 `native`
2. **给定** macOS / Linux，**当** 禁用原生 skill，**则** 同样进入系统回收站（跨平台一致行为）
3. **给定** 目标本身是 symlink / junction（非原生），**当** 禁用，**则** 直接移除链接（链接无数据可丢，不进回收站），Layer 2 / Layer 3 清理与 DB 更新行为不变
4. **给定** 回收站操作失败（网络盘、权限等），**当** 禁用原生 skill，**则** 返回错误提示，**绝不**自动降级为永久删除
5. **给定** 中英文界面，**则** 确认弹窗、结果 toast、i18n 键对齐，"移至回收站"文案与实际行为一致

### 用户故事 2 — 卸载资源端到端可用（优先级: P1）

**验收场景**：

1. **给定** 普通导入的 skill（DB id 为 `skill-<name>`，SSOT 目录为 `~/.mam/skills/<name>`），**当** 卸载，**则** 所有工具的链接与 assignment 清理、Layer 3 子 Agent 分配清理、SSOT 目录删除、`extensions` 与 `extension_assignments` 对应行删除、store 索引条目删除（如有）
2. **给定** manifest 安装的资源（DB id 为 `manifest.id`，SSOT 目录名为 `manifest.id`），**当** 卸载，**则** 路径解析正确，清理结果同场景 1
3. **给定** MCP 资源（SSOT 载体为 `~/.mam/mcp/<name>.json` **文件**），**当** 卸载，**则** 按资源 name（而非 ext_id）从所有工具配置移除该键，并删除 SSOT json 文件
4. **给定** config 型 plugin（tags 记录 `config`），**当** 卸载，**则** 调用 `disable_config_plugin` 移除 `plugins.<name>` 段，而非硬编码 file 型行为
5. **给定** 资源页任一行，**当** 打开操作菜单，**则** 存在「卸载」入口，二次确认弹窗提示将影响的工具数量
6. **给定** 子 Agent 有该 skill 的 Layer 3 分配，**当** 卸载，**则** 子 Agent 分配记录一并删除，无残留行

### 用户故事 3 — OpenCode JSONC 配置注释保留（优先级: P1）

**验收场景**：

1. **给定** `opencode.json` 含 `//` 或 `/* */` 注释，**当** 启用 / 禁用 MCP 或 config 型 plugin，**则** 注释与既有格式保留，仅 `mcp` / `plugins` 段对应键发生变化
2. **给定** 解析器无法安全定位编辑点（结构异常），**当** 操作，**则** 返回明确错误且**不写文件**
3. **给定** 带注释样本文件，**则** 单元测试覆盖启用→禁用往返后注释原样保留

### 用户故事 4 — 同名资源跨工具 / 跨类型不误伤（优先级: P1）

**验收场景**：

1. **给定** claude 与 codex 都装有同名 skill `foo`，**当** 首次自动导入，**则** SSOT 仅保留一份 `foo`，**两个工具**的 assignment 与工具目录链接都建立
2. **给定** skill `foo` 已导入，**当** 扫描到同名 plugin `foo`，**则** plugin 正常导入（去重集合按 kind 分离，不再被 skill 名吞掉）
3. **给定** 资源已存在，**当** 再次扫描到同名，**则** 不重复导入、不覆盖 SSOT 内容，`ImportStats` 计数准确

### 用户故事 5 — 增量发现新装 skill（优先级: P1）

**验收场景**：

1. **给定** 首次自动导入已完成（DB 非空），**当** 用户在 CLI 中新装 skill `bar` 后启动应用，**则** `bar` 被后台线程自动导入并为其来源工具启用，不阻塞启动
2. **给定** 某资源被用户显式禁用某工具（assignment `enabled=false`），**当** 增量扫描，**则** 不重新启用被禁用的 assignment
3. **给定** 手动「扫描原生资源」按钮，**当** 点击，**则** 维持现有 force 全量行为不变
4. **给定** 后台增量扫描进行中，**当** 用户同时操作资源开关，**则** 无冲突（增量扫描只处理 DB 中不存在的 name）

### 用户故事 6 — 断链检测与自动修复（优先级: P1）

**验收场景**：

1. **给定** Layer 2 / 工具目录链接 dangling（目标 SSOT 目录被外部删除），**当** 启动同步或 rescan，**则** SSOT 目录仍存在的重建链接；SSOT 也缺失的移除链接、assignment 标记 `missing`
2. **给定** `link_status` 新增 `dangling` 值，**当** 资源列表渲染，**则** 显示「损坏」徽标（i18n 中英文）
3. **给定** Windows junction 与 Unix symlink 两种链接形态，**则** 单元测试覆盖 dangling 判定与修复

### 用户故事 7 — 死代码清理与已写功能接线（优先级: P2）

**验收场景**：

1. **给定** 孤儿组件 `McpManager.tsx`，**当** 删除，**则** 构建通过，资源页 MCP 管理行为不变，其专用 i18n 键一并清理
2. **给定** 资源页工具栏，**当** 查看，**则** 存在「从 Manifest 安装」入口，走 `ManifestInstallDialog`（校验 → 权限 / 风险展示 → 安装）
3. **给定** `native_extensions` 表与 DAO（从未被业务调用），**当** 移除（含 migration `DROP TABLE IF EXISTS`），**则** 全部测试通过，无编译警告
4. **给定** 未使用的 React Query 封装（`useExtensionsQuery` / `useToggleMcpMutation`），**当** 本期完成，**则** `ResourceByKindView` 列表数据源接入 `useExtensionsQuery` 并在 toggle 后 invalidate；若实现受阻则删除封装并同步更新 ADR-004 状态（二选一，PR 说明中记录决策）

### 用户故事 8 — 扫描与安装安全加固（优先级: P2）

**验收场景**：

1. **给定** 超过 4 层深度的嵌套目录或 symlink 目录循环，**当** `scan_skills_recursive` 扫描，**则** 停止下探并记录跳过日志，不死循环
2. **给定** SSOT 已存在同名资源，**当** 手动安装（`install_skill` / plugin 安装），**则** 默认返回"已存在"错误；UI 弹确认后带 overwrite 重试成功；自动导入路径维持跳过语义（用户故事 4）
3. **给定** manifest `version` 字段，**则** semver 校验接受 `v` 前缀、三段版本、预发布与 build 元数据（如 `1.2.3-alpha.1+build.5`），非法值给出字段级错误

### 用户故事 9 — 批量操作与搜索补齐（优先级: P2）

**验收场景**：

1. **给定** 资源页搜索框，**当** 当前 kind 为 MCP / Plugin，**则** 搜索按名称与工具名过滤生效（现状仅 skill 可搜）
2. **给定** 资源行「全部启用 / 全部禁用」操作，**当** 执行，**则** 对 4 个工具顺序应用；禁用时 native skill 跳过并在结果中汇总提示（复用现有 native 警告流）；部分失败逐项汇总 toast
3. **给定** 中英文界面，**则** 新增文案 i18n 键对齐

## 设计

### 1. 禁用走系统回收站（P0）

- 引入 [`trash`](https://crates.io/crates/trash) crate（Windows 回收站 / macOS Trash / Linux XDG trash，跨平台单一 API）
- 重写 `commands/resource.rs:520-559` `disable_skill_for_tool`：
  - `target.is_symlink()`（含 junction）→ 维持 `linker::remove_link`（删链接本体，无数据损失）
  - native → `trash::delete(&target)`；失败返回 `Err`，**删除** `Command::new("trash")` 外部调用与 `Err` 分支的永久删除回退
  - 成功后的 Layer 3 清理、Layer 2 解链、`upsert_assignment(false, "missing")` 逻辑保持不变；返回值 `target_type` 语义不变

### 2. 重写 `uninstall_resource`（P1）

现状缺陷（`commands/manifest.rs:78-119`）：① `resource_dir` 统一按 `ext_id` 拼接，普通安装资源目录名是裸 `name`，路径必然找不到；MCP 的 SSOT 是 `<name>.json` 文件却被 `remove_dir_all`；② skill 分支把 `ext_id` 当 `skill_name` 传给 `services::skill::disable_skill_for_tool`（签名 `(skill_name, tool_id)`），实际清理全部落空且会写出 `skill-skill-xxx` 脏数据；③ MCP 分支把 `ext_id` 当 `mcp_name`；④ plugin 分支硬编码 `"file"`，config 型插件配置段不会被移除；⑤ 不删除 DB 行。

修复设计：

- 先经 `list_extensions()` 解析 `ext_id → ExtensionRecord`（kind / name / tags），不存在则报错
- SSOT 路径解析采用**双候选**规则（覆盖普通安装 `<name>` 与 manifest 安装 `<id>` 两种命名）：skill/plugin 目录候选 `[~/.mam/<kind>s/<name>, ~/.mam/<kind>s/<ext_id>]` 取第一个存在者；MCP 文件候选 `[~/.mam/mcp/<name>.json, ~/.mam/mcp/<ext_id>.json]`，用 `remove_file`
- 逐 assignment 清理一律用 **name**：skill → `services::skill::disable_skill_for_tool(name, tool)`；MCP → `services::mcp::remove_mcp(tool, name)`；plugin → 按 `record.tags`（`file` / `config`）走 `toggle_plugin(name, tool, false, tags)`
- 新增 DAO：`delete_extension(ext_id)`、`delete_assignments_for(ext_id)`（`dao/extension.rs`，含子 Agent 维度行，`database/mod.rs` 导出）
- `store::remove_entry(&ext_id)` 保留；条目不存在时忽略不报错
- 前端：`ResourceByKindView` 行操作菜单加「卸载」，调用已有封装 `src/lib/api/manifest.ts:20` `uninstallResource`；确认弹窗展示受影响工具数（数据源 `list_ssot_resources` 已含 assignments）

### 3. JSONC 文本级编辑保注释（P1）

- 引入 `jsonc-parser` crate（AST 节点带文本 range，可实现无损编辑）
- 改造 `services/mcp/mod.rs:134-162` `write_mcp_jsonc` / `remove_mcp_jsonc` 与 `services/plugin/mod.rs` `enable_config_plugin` / `disable_config_plugin` 的 Jsonc 分支（同一 `opencode.json`）：parse → 定位 `mcp` / `plugins` 对象与目标键的文本区间 → 字符串级替换 / 插入（新增键插入到对象末尾，按现有条目推断缩进）；`mcp` 段不存在时在 root 对象末尾追加
- 无法安全定位 → `Err("无法安全编辑该 JSONC 文件，请手动处理")`，不写文件
- 与 016 的关系：本项在 016-B（严格读取）之后实施，复用其读取 helper

### 4. 扫描去重按 kind 分离 + 同名补链（P1）

- `services/resource/mod.rs:209` 的 `seen_names` 拆为 `skill_seen` / `plugin_seen` 两个集合，消除 skill 名吞 plugin 名的跨 kind 误伤（`resource/mod.rs:306-309`）
- skill 循环内 name 已存在（seen 或 DB）时：跳过 `install_to_repo` 与 `insert_extension`，但**仍执行** `enable_skill_for_tool(name, tool_id)`——仅当该工具 assignment 不存在或 `enabled=true`（显式禁用的不碰），解决第二工具同名 skill 无链接问题

### 5. 启动增量导入（P1）

- `auto_import_extensions(force=false)` 从「DB 非空即整体返回」（`resource/mod.rs:188-199`）改为**增量模式**：仅处理 DB 中不存在的 name（导入 + 为来源工具启用）；已存在 name 完全不动 assignment
- `lib.rs:31-33` 的启动调用移入后台线程（`tokio::spawn` 或 `std::thread`），不阻塞启动；后台扫描与手动 rescan 互不阻塞
- 递归深度由用户故事 8 的限制兜底，控制扫描开销

### 6. 链接健康检测（P1）

- `linker` 新增 `check_link_health(target) -> LinkHealth { Valid, Dangling, NotLink, Missing }`（`symlink_metadata` + 目标 `canonicalize`/`metadata` 判定，兼容 junction）
- `list_layer2_skills` / `list_layer3_skills` 返回 `(name, health)`；调用方（含 `commands/resource.rs` 状态判定）适配
- `sync_imported_skill_links`（`services/resource/mod.rs:110-178`）扩展：dangling 且 SSOT 存在 → `remove_link` + 重建；SSOT 缺失 → `remove_link` + `upsert_assignment(…, "missing")`
- `link_status` 枚举扩展 `dangling`；`ResourceByKindView` 徽标与 i18n 适配

### 7. 死代码清理与接线（P2）

- 删除 `src/components/mcp/McpManager.tsx`（grep 确认无引用）
- `ResourceByKindView` 工具栏加「从 Manifest 安装」→ 挂载 `ManifestInstallDialog`（组件已实现校验 / 权限 / 风险展示，当前无页面引用）→ `validate_manifest` + `install_resource_from_manifest`；`src/lib/api/manifest.ts` 按需补齐封装
- 删除 `native_extensions`：`dao/extension.rs:179-236` 三函数、`database/mod.rs:21-22` 导出、`schema.rs:83` 建表语句；`migration.rs` 新增 `DROP TABLE IF EXISTS native_extensions`
- React Query 接线见用户故事 7 场景 4；`get_store_index` 与 `update_checker` 恒 None **本期不动**（见范围外）

### 8. 扫描 / 安装加固（P2）

- `scan_skills_recursive` 增加 `depth` 参数（上限 4）；仅 `entry.path().is_dir() && !entry.path().is_symlink()` 时下潜（防 symlink 循环）
- `install_to_repo`（`linker/mod.rs:126-163`）与 plugin 安装：`dest.exists()` 时默认返回含"已存在"字样的错误；新增 `overwrite: bool` 参数，commands 层透传，前端捕获错误弹确认后带 `overwrite=true` 重试；自动导入路径不传 overwrite（维持跳过）
- `services/manifest/validator.rs:146-149` 简化版 `is_valid_semver` 换为 `semver` crate：`Version::parse(version.trim_start_matches('v'))`，错误信息带字段定位

### 9. 批量操作与搜索（P2）

- `ResourceByKindView` 搜索过滤从仅 skill 扩展为当前 kind 全量（名称 + 关联工具名匹配）
- 行级「全部启用 / 全部禁用」：遍历 4 工具顺序 invoke；禁用 skill 前逐项调用 `checkSkillTargetType`，native 跳过并计数；结束后汇总 toast（成功 / 跳过 / 失败）

## 范围外

- GitHub ZIP 安装、content_hash 更新检查、marketplace UI（用户明确暂缓，`update_checker` stub 与 `get_store_index` 维持现状）
- 读取失败拒写、写前备份与回滚、per-tool 配置锁（→ `016-config-robustness-hardening`）
- React Query 全局架构替换（仅接线现有两个封装）
- 跨进程文件锁增强（`fs2` 现状够用）

## 测试策略

- Rust 单测（`cd src-tauri && cargo test`）：JSONC 带注释往返（用户故事 3）、去重与补链（故事 4）、增量导入语义（故事 5）、链接健康判定（故事 6）、semver 边界（故事 8）；涉及真实路径的测试用 `MAM_HOME`（debug 生效）或路径参数化的内部函数 + `tempfile`
- `cargo clippy` + `pnpm check`（含 i18n 键对齐）
- 手工验证：Windows 回收站可恢复（故事 1）；四类资源（普通 skill / manifest 资源 / MCP / config plugin）卸载后文件系统与 DB 无残留（故事 2）；批量启停与搜索（故事 9）
- 实施顺序建议：016-A/B/F（写路径基础设施）先行 → 本 spec P0/P1 → 016-C/D/E 并行收尾
