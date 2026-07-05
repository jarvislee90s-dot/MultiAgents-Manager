# 需求完整性检查清单：多 Agent 编程工具统一管理平台

**用途**：验证需求文档（spec.md / plan.md / tasks.md）的质量、完整性和一致性 — "英文单元测试"
**创建日期**：2026-07-05
**功能**：[spec.md](../spec.md) | [plan.md](../plan.md) | [tasks.md](../tasks.md)
**深度**：标准
**受众**：作者 + 审查者

## 需求完整性

- [ ] CHK001 - 是否定义了同一项目中运行多个同类工具会话的区分方式？（如 2 个 Claude Code 在同一目录运行时如何区分） [Gap, Spec §FR-1]
- [ ] CHK002 - 是否定义了会话异常退出（进程崩溃、终端强制关闭）时的看板行为？ [Gap, Spec §FR-1]
- [ ] CHK003 - 是否定义了最大并发监控会话数的上限行为？（超过 20+ 时是否降级轮询频率） [Gap, Spec §性能目标]
- [ ] CHK004 - 是否定义了 skill 从全局仓库卸载（删除原始文件）时的行为？已映射的工具如何处理？ [Gap, Spec §FR-5]
- [ ] CHK005 - 是否定义了全局仓库磁盘空间不足时的处理方式？ [Gap, Spec §FR-5]
- [ ] CHK006 - 是否定义了预设组的最大成员数量限制？ [Gap, Spec §FR-6]
- [ ] CHK007 - 是否定义了子 Agent 的发现机制（扫描哪些目录、什么格式）？ [Gap, Spec §FR-7]
- [ ] CHK008 - 是否定义了不支持子 Agent 的工具（如 Claude Code 的子 Agent 是可选的）的行为？ [Gap, Spec §FR-7]
- [ ] CHK009 - 是否定义了应用的自动更新需求和策略？ [Gap]
- [ ] CHK010 - 是否定义了日志和调试信息的保留策略？ [Gap]

## 需求清晰度

- [ ] CHK011 - 通知优先级是否明确？（同时多个会话状态变更时通知的排序和合并策略） [Clarity, Spec §FR-3]
- [ ] CHK012 - 不同状态的默认提示音是否明确指定了具体频率或音效名称？ [Clarity, Spec §FR-3]
- [ ] CHK013 - "状态变更后 3 秒内收到通知"是否区分了 Hook 工具（Claude/Codex）和无 Hook 工具（OpenCode）的延迟差异？ [Clarity, Spec §成功标准2]
- [ ] CHK014 - "红绿灯"是否定义了五态（Waiting/Processing/Thinking/Compacting/Idle）各自对应的颜色和动画？ [Clarity, Spec §FR-2]
- [ ] CHK015 - MCP 服务器配置的内部统一格式（{command, args, env}）是否在 spec 中明确定义？还是仅在 plan/data-model 中？ [Clarity, Spec §FR-5]
- [ ] CHK016 - "预设组应用到不同工具时自动适配格式"是否明确说明了适配的具体行为（如 TOML→JSON 的字段映射）？ [Clarity, Spec §FR-6]
- [ ] CHK017 - "子 Agent 只能使用工具级已启用的资源范围内的子集" — "子集"是否明确为"不能新增、只能筛选"？ [Clarity, Spec §FR-7]
- [ ] CHK018 - Wayland 降级提示的具体用户可见文案是否定义？ [Clarity, Spec §FR-4]

## 需求一致性

- [x] CHK019 - FR-1 第 1 条已更新为三工具 MVP（Claude Code + Codex CLI + OpenCode），与成功标准一致 [已解决]
- [ ] CHK020 - FR-6 第 25 条"同一预设组应用到不同工具时自动适配格式"与 FR-5 第 18 条"按目标工具格式写入"是否描述同一行为？是否存在重复或矛盾？ [Consistency, Spec §FR-5 vs §FR-6]
- [ ] CHK021 - 工具级新增资源时子 Agent 是否自动获得？ [已解决] — 子 Agent 默认不包含新增资源；skill 仅通过预设组分配，工具级新增 skill 后需显式将预设组应用到子 Agent
- [ ] CHK022 - 用户故事 5 的预设组包含"skill + MCP + 插件"，但 plan.md 的预设组应用逻辑只描述了 skill（symlink）和 MCP（配置写入）— 插件的预设组应用逻辑是否完整？ [Consistency, Spec §US5 vs Plan §预设组应用逻辑]
- [ ] CHK023 - data-model.md 的 session_status_cache 表用于"通知去重"，但 plan.md 通知去重逻辑写的是"5 秒内不重复" — SQLite 持久化和内存去重的关系是否一致？应用重启后是否需要重发通知？ [Consistency, data-model vs Plan]
- [ ] CHK024 - constitution v1.2.0 阶段二写"增加更多工具"，但之前的阶段三/四内容被合并了 — 多 Agent 编排、移动端、自愈机制是否还在需求范围内？ [Consistency, Constitution vs Spec]

## 验收标准质量

- [ ] CHK025 - "应用启动时间不超过 3 秒"是否区分了冷启动（首次安装后）和热启动（已有缓存）？ [Measurability, Spec §成功标准6]
- [ ] CHK026 - "状态轮询不影响系统其他操作的流畅度（CPU 占用 < 5%）" — 这个 5% 是指应用进程的 CPU 还是系统总 CPU？ [Measurability, Spec §成功标准7]
- [ ] CHK027 - "一键切换可在 5 秒内完成整组 skill 的启用/禁用" — 是否考虑了 MCP 配置写入（文件 I/O）的时间？大型预设组（20+ 项）是否也能在 5 秒内？ [Measurability, Spec §成功标准5]
- [ ] CHK028 - "会话状态变更后 3 秒内收到通知"是否可验证？如何触发和测量这个 3 秒窗口？ [Measurability, Spec §成功标准2]

## 场景覆盖率

- [ ] CHK029 - 是否定义了通知疲劳场景？（短时间内 10+ 会话同时状态变更时的处理） [Coverage, Gap]
- [ ] CHK030 - 是否定义了勿扰模式的需求？（用户不希望被打扰时的行为） [Coverage, Gap]
- [ ] CHK031 - 是否定义了应用首次启动的引导流程？（无会话、无 skill 时的空状态） [Coverage, Gap]
- [ ] CHK032 - 是否定义了配置导入/导出的需求？（用户换机或备份时） [Coverage, Gap]
- [ ] CHK033 - 是否定义了多语言/国际化的需求？（tauri-app-template 已预置 i18n，但 spec 未提及） [Coverage, Gap]
- [ ] CHK034 - 是否定义了 Codex 桌面 APP 与 CLI 同时运行时的会话区分？（两者共享 ~/.codex/） [已解决] — CLI 进程名 codex / APP 进程名 Codex，通过进程名区分为不同会话；APP 标记 form=App, jump_supported=false
- [ ] CHK035 - 是否定义了同一工具的不同版本（如 Claude Code v2.1 vs v2.2）的兼容性处理？ [Coverage, Gap]

## 边界条件覆盖

- [ ] CHK036 - 是否定义了 Hook 脚本执行失败时的回退行为？（权限问题、脚本 bug、路径不存在） [Edge Case, Spec §FR-1]
- [ ] CHK037 - 是否定义了 JSONL 文件损坏或格式异常时的降级行为？ [Edge Case, Spec §FR-1]
- [ ] CHK038 - 是否定义了 symlink 创建失败（权限不足、路径冲突）时的错误提示和替代方案？ [Edge Case, Spec §FR-5]
- [ ] CHK039 - 是否定义了 MCP 配置写入失败（文件被占用、格式错误）时的回滚策略？ [Edge Case, Spec §FR-5]
- [ ] CHK040 - 是否定义了预设组中某项资源安装失败时的部分成功处理？ [已解决] — 保留已成功的项，失败项报告给用户手动处理，不自动回滚（spec FR-6 第 27 条 + plan 部分成功处理章节）
- [ ] CHK041 - 是否定义了工具升级后配置格式变化时的兼容处理？（如 Codex config.toml schema 变更） [Edge Case, Gap]
- [ ] CHK042 - 是否定义了 OpenCode storage 目录不存在或为空时的行为？ [Edge Case, Spec §FR-1]

## 非功能需求

- [ ] CHK043 - CPU 占用 <5% 的目标是否区分了 idle（无活跃会话）和 active（20 会话轮询）场景？ [Completeness, Spec §性能目标]
- [ ] CHK044 - 是否定义了内存使用的目标？（Tauri + WebView + Rust 后端的总内存上限） [Gap]
- [ ] CHK045 - 是否定义了安全扫描的具体规则范围？ [已解决] — 仅检查 skill 文件路径是否落在允许目录内（防路径穿越），不检查文件内容（spec FR-8 第 34 条）
- [ ] CHK046 - 是否定义了 MCP 服务器命令的安全审查需求？（MCP 可执行任意命令，仅展示权限摘要是否足够？） [Completeness, Spec §FR-8]
- [ ] CHK047 - 是否定义了首次启用 MCP 服务器时的确认对话框的具体内容？ [Clarity, Spec §FR-8]

## 依赖与假设

- [ ] CHK048 - 假设第 4 条"支持的工具会持续更新其 Hook/日志/MCP 格式" — 是否定义了格式变更时的适配流程和版本兼容性策略？ [Dependency, Spec §假设]
- [ ] CHK049 - 假设第 1 条"用户已安装至少一种工具" — 是否定义了未安装任何工具时的引导行为？ [Assumption, Spec §假设]
- [ ] CHK050 - research.md 中标注"Codex CLI Hook stdin 格式需在编码前验证" — 这个验证是否已纳入 tasks.md 的任务？ [Dependency, Research §待验证项]

## 歧义与冲突

- [ ] CHK051 - "Finished" 状态在 spec 中用于"进程不存在"和"会话结束"两种含义 — 是否应区分"正常结束"和"异常退出"？ [Ambiguity, Spec §FR-1]
- [ ] CHK052 - 用户故事 6 验收场景 3"预设组中的 skill 必须在工具级范围内" — "范围内"是指"已启用"还是"已安装到全局仓库"？ [Ambiguity, Spec §US6]
- [ ] CHK053 - plan.md 通知去重写"5 秒内不重复"，但未定义 5 秒后是否重新通知 — 如果状态一直不变，是否持续通知？ [Ambiguity, Plan §通知去重]
- [x] CHK054 - Skill 不支持单独启用，只能通过预设组，故取消激活时无手动启用的 skill 冲突；MCP/插件按冲突处理（保留已有、跳过） [已解决]
