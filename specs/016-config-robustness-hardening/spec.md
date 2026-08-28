# 功能规格说明：配置写入健壮性加固（写前备份 + 拒写保护 + DB 备份 + MCP 校验 + 漂移检测 + 工具配置锁）

**功能分支**：`016-config-robustness-hardening`

**创建日期**：2026-08-25

**状态**：草稿

**输入**：与 cc-switch（main 分支）对比结论——cc-switch 对"覆写用户配置文件"的操作有五层防护（原子写、写前备份与回滚、配置校验、每 app 操作锁、切换前 backfill 回填），本项目目前只有原子写一层（`linker::write_config_locked`，temp + rename + 文件锁）。本 spec 落地六项加固，均借鉴 cc-switch 的对应机制：①写前备份 + 设置页回滚入口；②读取/解析失败绝不以空对象覆写；③SQLite 自动备份（迁移前 + 周期）；④MCP 单条校验；⑤配置漂移检测（backfill 思想）；⑥每工具配置互斥锁。
GitHub ZIP 安装与 content_hash 更新检查**不在本期**（用户明确暂缓）。

## 用户场景与测试

### 用户故事 1 — 写前自动备份与回滚入口（优先级: P0）

**验收场景**：

1. **给定** 任一走 `write_config_locked` 的写入（MCP 启停、config 型 plugin 启停、hook 注册），**当** 写入发生，**则** 写入前的旧内容自动备份到 `~/.mam/backups/config/<目标文件标识>/<时间戳>.bak`，每个目标文件保留最近 10 份
2. **给定** 设置页新增「配置备份」区块，**当** 查看，**则** 按目标文件分组列出备份（文件、时间、大小），每条支持「恢复」与「删除」
3. **给定** 恢复操作，**当** 执行，**则** 二次确认后恢复内容并走原子写；恢复前自动备份当前内容（恢复本身可再撤销）
4. **给定** `monitor/hooks.rs:137-141` 原有的 `settings.json.bak` 独立备份逻辑，**当** 本期完成，**则** 统一迁移到同一备份机制，不再各自为政
5. **给定** 备份目录不可写等备份失败，**当** 主操作写入，**则** 记录 warning 日志但主写入不受阻断

### 用户故事 2 — 读取/解析失败绝不以空对象覆写（优先级: P0）

**验收场景**：

1. **给定** 工具配置文件存在但读取失败（权限、被锁、非 UTF-8 等非 NotFound 错误），**当** 任一 MCP / Plugin 写操作，**则** 返回错误拒绝写入，文件字节不变
2. **给定** 工具配置文件不存在（NotFound），**当** 启用 MCP，**则** 允许起草新文件（现状合法场景不受影响）
3. **给定** 文件存在但 JSON / TOML 语法损坏，**当** 写操作，**则** 返回解析错误不写入（现状已如此，补测试固化防回归）
4. **给定** `import_mcp_to_ssot`（其输出将成为 SSOT 内容），**当** 读取工具配置，**则** 使用严格读取，失败即报错，不落半空配置进 SSOT

### 用户故事 3 — 数据库自动备份（优先级: P1）

**验收场景**：

1. **给定** 任意一次启动，**当** schema migration 执行前，**则** 先做完整备份（`VACUUM INTO`，含 WAL 内容的一致性快照）到 `~/.mam/backups/db/mam_<时间戳>.db`
2. **给定** 距最近一次 DB 备份超过 24 小时，**当** 应用启动，**则** 后台线程补一次备份，不阻塞启动
3. **给定** 备份目录超过 10 份，**当** 新备份产生，**则** 自动清理最旧
4. **给定** 设置页「配置备份」区块，**当** 查看 DB 备份列表，**则** 支持「在文件管理器中显示」与「复制到指定位置」；不提供应用内一键还原（进行中替换自身 DB 有风险，界面给出"关闭应用后手动替换 ~/.mam/mam.db"的指引文案）

### 用户故事 4 — MCP 单条校验（优先级: P1）

**验收场景**：

1. **给定** MCP 添加 / 编辑表单，**当** command 为空或纯空白，**则** 前端即时红错、禁止提交
2. **给定** command 在 PATH 中找不到（含 Windows PATHEXT 探测），**当** 提交，**则** 警告确认后允许继续（兼容 npx / 自定义路径等误报场景）
3. **给定** 后端 `toggle_mcp` / `save_mcp_config`，**当** 写入前，**则** 复用同一校验：错误拒绝写入并返回明细，警告放行并记日志
4. **给定** 前端表单，**则** 通过 `validate_mcp_config` IPC 复用后端规则，前后端校验一致

### 用户故事 5 — 配置漂移检测（backfill 思想）（优先级: P1）

**验收场景**：

1. **给定** DB 记录 `enabled=true` 但工具配置中已无该 MCP 键 / skill 链接缺失（用户手删），**当** 启动同步或 rescan，**则** 产生漂移项并报告
2. **给定** DB 记录 `enabled=false` 但工具配置中存在同名 MCP 键、或工具 skill 目录存在非链接同名目录（用户手装），**当** 检测，**则** 产生「未纳管资源」漂移项（后者归并现有 `detect_duplicate_skills` 结果）
3. **给定** 资源页，**当** 存在漂移，**则** 顶部横幅「检测到 N 处配置漂移」，点开逐条展示：资源、工具、期望状态 vs 实际状态
4. **给定** 单条漂移，**当** 用户选择，**则** 两个动作：「以 MAM 为准」（重建链接 / 回写配置键 / 移除多余键）与「以工具为准」（导入 SSOT + 更新 assignment 为现状）
5. **给定** 启动后台同步，**当** 执行，**则** 仅"断链且 SSOT 仍在"这一安全项自动修复（联动 015 用户故事 6），其余漂移只报告、绝不自动改动

### 用户故事 6 — 每工具配置互斥锁（优先级: P1）

**验收场景**：

1. **给定** 快速连续切换两个不同 MCP 到同一工具，**当** 两个 IPC 并发到达，**则** 两次键级合并都生效，互相不覆盖（现状 read 在锁外，可能丢更新）
2. **给定** 预设组应用与手动开关并发写同一工具配置，**当** 同时发生，**则** 串行化执行，最终状态一致无丢失
3. **给定** 锁实现，**则** 按 tool_id 粒度（claude / codex / opencode / openclaw 各一把），不同工具的操作不互相阻塞；预设循环按固定顺序 `claude → codex → opencode → openclaw` 获取，无死锁
4. **给定** 并发场景，**则** 单元测试验证：两线程并发对同一临时配置文件写入不同 MCP 键，最终两键共存
5. `write_config_locked` 的文件锁保留，作为跨进程（外部工具同时写同一配置）的最后防线

## 设计

### 1. 写前备份 + 回滚（P0）

- `linker/mod.rs:175-196` `write_config_locked` 增强：获取文件锁后、写 temp 前，若 `path.exists()` 则 `fs::copy` 旧内容到 `~/.mam/backups/config/<sanitized>/<YYYYMMDD-HHMMSS>-<seq>.bak`（`sanitized` 为目标文件全路径中 `\\ / :` 替换 `_`，同秒冲突加序号）；随后按文件清理超出 10 份的最旧备份
- 备份失败 `log::warn` 后继续主写入（主功能可用性优先）；备份本身不需要加锁（在已持有的文件锁内执行）
- hook 写入路径改为统一走 `write_config_locked`（自动获得备份），移除 `monitor/hooks.rs:137-141` 的独立 `.bak`
- 新增 IPC：`list_config_backups() -> Vec<ConfigBackup>`（key / 文件名 / 时间 / 大小）、`restore_config_backup(key, ts)`（读 `.bak` → `write_config_locked` 回原路径，天然再次备份当前内容）、`delete_config_backup(key, ts)`；在 `commands/` 注册并加入前端 API 封装
- 设置页新增「配置备份」卡片：配置文件备份（分组 + 恢复 + 删除）与 DB 备份（列表 + 打开目录 + 复制到…）两个分区；新增 i18n 键（中英文）

### 2. 严格读取（P0）

- `linker` 新增 `read_config_exact(path) -> Result<Option<String>, String>`：`ErrKind::NotFound` → `Ok(None)`；其他 IO 错误 → `Err`（含错误上下文）。调用方对 `None` 起草 `"{}"`（JSON）/ 空串（TOML）
- 替换所有写路径的宽松读取：`services/mcp/mod.rs:57,73,121,135,154`（write/remove × JSON/TOML/JSONC 共 6 处）、`services/plugin/mod.rs:137,153,226,236`（enable/disable_config_plugin）
- `commands/resource.rs:570-628` `import_mcp_to_ssot` 的工具配置读取改用 `read_config_exact`；仅用于**展示**的读取（列表扫描类）维持宽松
- 解析失败路径现状已返回 Err（保持），补单测固化

### 3. 数据库自动备份（P1）

- `database::init`（`database/mod.rs:33-38`）在 `migration::migrate` 之前：`conn.execute("VACUUM INTO ?1", [backup_path])`（rusqlite 参数绑定），目标 `~/.mam/backups/db/mam_<YYYYMMDD-HHMMSS>.db`；`VACUUM INTO` 产出包含 WAL 内容的一致性快照，无需 checkpoint
- 备份失败 `log::error` 不阻断启动 / 迁移（迁移本身幂等，风险可控）
- 周期备份：`lib.rs` 启动后台线程检查 `~/.mam/backups/db/` 最新 mtime，超过 24h 则经 `DB` Mutex 执行一次 `VACUUM INTO`
- 清理策略同配置备份：保留最近 10 份

### 4. MCP 单条校验（P1）

- `services/mcp` 新增 `McpValidation { errors: Vec<String>, warnings: Vec<String> }` 与 `validate_mcp_config(&McpConfig)`
- Error：`command.trim()` 为空
- Warning：`find_in_path(command)` 为 None。新增公共函数 `find_in_path`（扫 `PATH`；Windows 按 `PATHEXT`（`.exe/.cmd/.bat/.ps1`）逐一探测；command 含路径分隔符时按绝对 / 相对路径 `exists()` 判断；`npx`/`uvx`/`bunx` 这类 runner 本身在 PATH 即不告警）
- 接入点：`services::mcp::write_mcp` 入口与 `commands/resource.rs` `save_mcp_config` / `toggle_mcp`——errors 非空返回 Err（附明细），warnings 非空 `log::warn` 放行
- 新增 IPC `validate_mcp_config`；`ResourceByKindView` MCP 弹窗（`ResourceByKindView.tsx:393-456`）提交前调用：错误阻断、警告确认

### 5. 漂移检测（P1）

- 新增 `services/drift.rs`：`DriftItem { ext_id, kind, name, tool_id, drift_type, detail }`；`drift_type ∈ { LinkMissing, LinkDangling, RepoMissing, McpKeyMissing, UnmanagedMcpKey, UnmanagedSkillDir }`
- `detect_drift()` 数据源：DB assignments（enabled）+ 文件系统核验（链接健康（015 的 `check_link_health`）、SSOT 目录存在性、工具配置键存在性——检测用读取可宽松）
- 执行时机：启动后台线程在 `sync_imported_skill_links` 之后运行并缓存结果；「扫描原生资源」rescan 后刷新
- IPC：`list_drift()`、`resolve_drift(item_id, action)`：
  - `MamFirst`：LinkMissing / LinkDangling → `enable_skill_for_tool`；McpKeyMissing → 重新 `write_mcp`；UnmanagedMcpKey → `remove_mcp`；UnmanagedSkillDir → 复用现有"替换为链接"清理
  - `ToolFirst`：UnmanagedMcpKey → `import_mcp_to_ssot` + `upsert_assignment(enabled=true)`；UnmanagedSkillDir → 走现有导入；McpKeyMissing / LinkMissing → `upsert_assignment(enabled=false)`（承认现状）
- 前端：`ExtensionList` 顶部横幅 + 抽屉面板逐条渲染与操作；「全部以 MAM 为准 / 全部以工具为准」批量按钮；i18n
- 自动修复白名单仅一项：断链且 SSOT 仍在（与 015 用户故事 6 的启动行为一致），其余一律人工决策

### 6. 每工具配置锁（P1）

- 新增 `linker/tool_config_lock.rs`：`static LOCKS: Lazy<DashMap<String, Arc<Mutex<()>>>>`（`dashmap` 与 `once_cell` 均已在依赖中）；`pub fn with_tool_config_lock<T>(tool_id: &str, f: impl FnOnce() -> T) -> T`（`entry().or_default` clone Arc 后 `lock()`）
- 包裹点（完整覆盖 read → modify → write 序列，当前 read 在文件锁之外是竞态根源）：
  - `services::mcp::write_mcp` / `remove_mcp`（`get_tool_mcp_info` 已解析 tool_id）
  - `services::plugin::enable_config_plugin` / `disable_config_plugin`
  - `monitor::hooks` 注册写入
  - preset 应用循环内每工具段（固定顺序 `claude → codex → opencode → openclaw`）
- 不跨工具持锁（单工具粒度），无锁序死锁风险；文件锁保留为跨进程最后防线
- 单测：`tempfile` 目标文件 + 两个线程经锁包装并发调用 `write_mcp_json(path, "a"/"b")`，断言两键共存

## 范围外

- GitHub ZIP 安装、content_hash 更新检查、资源版本管理（用户明确暂缓）
- 云同步 / WebDAV、多设备配置分发
- DB 应用内一键还原（仅提供列表 + 打开目录 + 复制导出）
- OpenCode JSONC 保注释编辑（→ `015-resource-management-fixes` 用户故事 3，在本文档 §2 严格读取之上实施）
- 外部进程写配置的实时文件监听（`notify` 已在依赖中，本期不做 watch）

## 测试策略

- Rust 单测（`cd src-tauri && cargo test`）：备份产生与保留上限（用户故事 1）、`read_config_exact` 的 NotFound / 其他错误分支（故事 2）、`VACUUM INTO` 备份可打开且表完整（故事 3）、`validate_mcp_config` 空 command / PATH 探测（故事 4）、`detect_drift` 各漂移类型构造（故事 5）、并发写入两键共存（故事 6）；涉及真实路径的用 `MAM_HOME`（debug 生效）或路径参数化内部函数 + `tempfile`
- `cargo clippy` + `pnpm check`（含 i18n 键对齐）
- 手工验证：启用 MCP 后从设置页恢复备份，`~/.claude.json` 字节级回到启用前（故事 1）；人为制造漂移（手删配置键 / 手装 skill）后横幅出现且双向决策正确（故事 5）；快速连点两个 MCP 开关无丢失（故事 6）
- 实施顺序建议：本文档 §1 / §2 / §6（写路径基础设施）先行 → 015 各项 → 本文档 §3 / §4 / §5 并行收尾
