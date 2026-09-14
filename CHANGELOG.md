# Changelog

## [0.4.0] - 2026-09-14
### Added
- **跳转 marker 通道（Windows，issue #43）**：点击卡片时经 `mam-marker` helper（随应用自动分发至 `~/.mam/bin/`）对目标会话终端贴一次性身份标记 ` — MAM:<session_id 剥连字符前 12 位>`（官方控制台 `SetConsoleTitle` 通道优先 + 近祖窗口标题直改兜底），窗口消歧①层精确命中；聚焦成功后自动清痕回干净态、注入前剥旧防叠加（不同会话先后跳同一窗不残留双标记）；helper 缺失/失败全链静默回落既有消歧层（零回归）
- **Hook 事件通道重构（issue #43）**：claude/codex 状态 hook 事件文件改以 session_id 为键（原 PPID 键在多会话下互覆），写入前字符白名单校验（防路径注入，合法 UUID 永不触发）；读取侧 30s TTL；claude 的 Stop 宽限（防误报红灯）恢复生效，周期性 marker 注入块退役
- **ZCode 第七工具支持**（智谱桌面 AI 编程助手，Electron APP 形态）：宿主判定（进程侧只回答「应用开没开」，Windows 全部可执行体同名 ZCode.exe 时按命令行区分主进程/辅助进程/会话运行时）+ 双 SQLite 会话聚合（tasks-index 任务索引 + session/message/part 消息流，每会话一卡、24h 窗口、archived/deleted 过滤、会话 id 严格 UUID 校验）；状态从消息流尾部按 sequence 倒扫推导（step-finish 懒落库时间戳不可靠、顺序可靠），todo_reminder 等记账消息识别跳过；子代理长任务经共享层「后代活跃度仲裁」保持运行中（主会话静默 + 子会话活跃 = 健康等待，停更且无后代活动 = 疑似卡住）；task_status=error 按完成转绿。资源管理：skill 分发（SSOT → `~/.zcode/skills/` 建链/删链，**目录已实测被 ZCode 识别**）+ MCP 读-改-写（`~/.zcode/cli/config.json` 仅 `mcp.servers` 子树，未知键与原键序保留，解析失败只报错不落盘，`enable:false` 如实展示为停用）
- **ZCode 跳转 = 直接聚焦唯一窗口**：ZCode 为单窗口多标签应用（实机取证：`windowId:1` 恒定、标题恒为 "ZCode" 不含工作区名），点击卡片经 pid 单窗口路径零歧义锁定，失败落 APP 级激活兜底。工作区深链 `zcode://workspace/open` 经实机验收后**有意不接入**：其语义为打开工作区 + 全新会话 composer（非定位已有会话）、每次派发无条件弹信任确认且拉起一个转发进程
- **同项目双开跳转直达（窗口标题匹配层）**：Kimi / OpenCode 的终端窗口标题与会话标题（kimi `state.json` 标题 / OpenCode DB 标题）归一化比对（剥 spinner 前缀 / "OC | " 工具前缀 / 尾部省略号），唯一命中即锁定——双开终端不再弹选择器；会话标题经全部跳转入口（看板 / 通知 / 桌宠 / 历史）统一透传
- 卡片项目名显示预算：中文封顶 10 字、英文封顶 15 字符（15 单位宽度预算，CJK 每字 1.5 单位），截断点确定、不随卡宽漂移，完整名悬停可见
- 共享层通用化（D5）：停更降级新增后代活跃度仲裁参数（空后代语义 = 现状，既有工具行为零变化并有逐边界回归测试）；数据驱动持久绿卡门（P1-3 剔除 + 未读态标记）从 Codex 特判泛化为工具能力判定，ZCode 聚合卡复用同款语义
- **codex 私有 Skill 目录化**：codex 的 skill 激活目录从跨工具共享的 `~/.agents/skills/` 切换为私有 `~/.codex/skills/`（codex 0.149.1 实测读取该目录，官方捆绑系统技能同落此处）——对 codex 单独启用/禁用技能不再泄露给 zcode 等遵循 Agent Skills 开放标准的工具
- **`~/.agents/skills/` 转为只读共享导入源（来源标签 `agents-shared`）**：手装技能照常自动扫描入库，但不归属任何工具、不建链；除一次性迁移 MAM 自建遗留链接外，MAM 不再向该目录写入（codex / zcode 等遵循开放标准的工具直接读取）
- **遗留 codex 链接一次性迁移对话框**：启动时后台检测 `.agents/skills` 下 MAM 自建链接（指向 `~/.mam/active/codex/`），命中即弹二选一——「迁移到 `.codex/skills`」（DB 分配与 Layer 2 不动，启停状态无损）或「保留为共享」（改指 SSOT 仓库 `~/.mam/skills/`，脱钩 codex 启停、继续服务未适配工具）；两种选择后对话框均自熄灭不再触发
- **SSOT 删除保护**：删除仍被 `~/.agents/skills/` 直链引用的 SSOT 技能时弹出确认提示，防止依赖共享目录的未适配工具失效
- **Codex APP 会话 SQLite 适配**（新版 Codex 桌面端已把会话存储从 `~/.codex/sessions/*.jsonl` 迁入 SQLite——实机取证 state_5 库含 rollout_migration_state 迁移表，rollout 目录 9 月 3 日后零新写入，看板因此检测不到 APP 对话）：APP 卡改由 `monitor/codex_thread_parser` 双库产出——`state_*.sqlite` 的 `threads` 表（cwd/标题/git 分支/git 远端/updated_at(秒)/archived/source）为实时元数据源，`thread_history_*.sqlite` 的 `thread_items`/`thread_turns` 内容投影**可用则用**（remote_control 通道的活跃对话不落本地投影，按 threads.updated_at 新鲜度兜底 + 共享核 300s 停更降级）；turn 生命周期（inProgress→Processing 强信号、failed→Finished 转绿）与 `thread_spawn_edges` 子代理树（后代活跃度仲裁 + 「N 个子代理」）直取 DB。文件名带版本号按数值取最新（state_5→state_6 升级不破）。rollout 线路保留给 CLI 前端（双源），CLI 认领的会话不再重复出 APP 卡；旧 rollout 聚合链（aggregate_app_sessions 等）随存储迁移退役

### Changed
- **配对不确定安全门（issue #48）**：同工具同项目 ≥2 进程时，卡片↔终端启发式配对可能互换——禁用一切演绎锁定（单窗口即锁/幸存者推理）与按需 marker 注入（防贴错窗后自证锁错），只认正向证据、素材不足交选择器；卡片加「同项目双开」提示角标（诚实文案，不承诺跳转行为）
- **同标题双命中 UIA 仲裁（issue #49）**：窗口标题匹配层命中 ≥2 时，把 UIA 尾串包含匹配前移到命中集合仲裁，唯中者锁定、仍不唯一落选择器（宁弹不锁错）
- **聚焦失败诚实报错**：置前被系统拒绝（锁屏 / 前台锁）时返回错误并 toast 显式提示，不再静默假成功
- **选择器零分候选视觉弱化**：score=0 且无 UIA 命中的候选降为 60% 不透明度，排序保持后端降序不过滤（防误杀评分素材缺失的真目标）
- 命令层统一 `AgentType::tool_id()` 显式映射，收口 6 处 `format!("{:?}")` 隐式推导（新增变体编译期强制补齐）
- **会话扫描三层预算（性能）**：前端 3 秒轮询下各解析器不再每轮全量重扫历史会话（实机 codex 会话库 2GB / 388 文件曾把主线程打满 100%、界面卡死）。三层通用机制落在 `monitor/session_scan`：① 零进程零解析（`get_all_sessions` 编排层统一短路 + 全 adapter 防回归测试）；② `(mtime,size)` 内容摘要缓存——解析拆「纯内容产物（缓存）+ 时间叠加（现算）」，codex / claude / kimi / workbuddy 已接入；③ 无界历史扫描限 24h 新鲜窗口 + 活跃进程窗口内匹配不到才回退全量（codex；进程界定有界扫描不窗口化以免丢空闲超窗的活跃卡）。opencode / zcode（SQLite 查询即过滤）与 openclaw（配置界定）仅享 ①。契约已写入 AGENTS.md「Agent Adapter 模式」与 trait 文档，新工具接入自动受保护
- **codex / zcode 停更不落兜底红灯**：无内容信号时无法区分「等用户输入」与「对话已结束」，时间兜底一律落绿灯（Idle 完成待看，接入既有未读/绿卡管线）——codex rollout 路线的无信号兜底与 300s 停更降级、zcode 的无信号 fallback 均已改；红灯只保留给有证据的等待（claude 内容判定、zcode 内容+后代仲裁「疑似卡住」）；进程绑定型工具（claude/kimi/workbuddy）不动

### Fixed
- **macOS universal 构建兼容**：`mam-marker` helper bin 加 `marker-helper` feature 门控——macOS universal 打包只对主二进制 lipo，额外 bin 会令打包必然失败；helper 仅随 Windows 安装包分发（Windows-only 功能），macOS 构建不再产出该 bin
- **空壳终端窗口混入跳转选择器（issue #47）**：`C:\WINDOWS\system32\cmd.exe `（全路径 + 尾空格）形态的空闲终端按 basename 归一化命中排除名单，不再成为跳转候选；名单比对统一 trim + 小写
- **系统通知跳转 title 漏传（issue #45）**：系统通知入口此前不传会话标题、标题匹配层对该入口永不生效——浮窗 / 系统 toast / 铃铛历史全部路径补齐（空值安全回落，无假命中面）
- **CI windows-gnu 交叉编译门禁（issue #45）**：`cfg(windows)` 代码进 CI（`cargo check` + `cargo clippy -D warnings` 双步），Windows 侧编译/告警问题不再漏检
- **窄卡标题尾巴阈值（issue #51）**：容器查询按 content box 度量校准，外卡约 400px 以下会话标题尾巴整段隐藏（不留孤立省略号），项目名不再被挤压
- **zcode 集成加固（issue #52）**：宿主判定加命令行路径门控 + 后代进程黑名单（防辅助进程误判宿主）；缺排序列的会话整卡跳过（不出错误卡）；MCP 写路径与读路径对称；`enable:false` 的 MCP 条目导入保留并显示「源已停用」角标
- **claude hook 命令注册修复**：无空格路径去引号（bash 把引号当字面量）+ 正斜杠形态；存量双形态条目迁移谓词修复，SessionStart 不再报错
- **Kimi 双开跳转必弹选择器**：CLI 写入 `state.json` 的 `updatedAt` 为整数毫秒，serde 类型不匹配导致整个状态解析失败 → 会话标题回退 `session_` 前缀 → 标题匹配层永不命中；现兼容数值 / 字符串双形态
- **OpenCode 同目录双终端只显示一个会话**：两个进程都认领最新一条会话行、旧会话被遮蔽；现按目录配对到各自进程（已配对行防复用）
- Codex 会话标题前缀 8 → 12 位，消除同一分钟内创建会话的键碰撞
- 窗口标题匹配层空标题守卫（归一化后的空串是任何键的「前缀」，会假命中）
- **深链派发改 ShellExecuteW 优先**：`cmd /C start` 会把成对 `%` 当环境变量展开、破坏 percent-encoded URL（对经同一入口派发的 WorkBuddy/Codex 深链同样生效，cmd 降为兜底）
- ZCode 卡片：子代理计数只数 30 秒活跃窗口内的子行（原为终身总数按最新行门控，先后派生多个子代理时计数虚高）；最后消息摘要跳过 `<todo>` 等记账消息（原会把工具内部提醒当对话内容展示且角色误标 user）
- MCP 面板读取链路硬编码旧工具清单（仅 claude/codex/opencode，openclaw/kimi/workbuddy 的 MCP 读取一直落「未知工具」）——改走 adapter 注册表统一分发，段定位按 adapter 声明的键路径（兼容历史顶层键探测）；资源导入溯源 `detect_source_tool` 与前端 `SUPPORTED_TOOLS` 硬编码清单同批注册表化

### Changed
- 会话卡顶行布局：项目名优先完整显示，会话标题压缩为灰色尾巴、空间不足时先让路（此前 kimi 长任务标题曾把项目名挤成单字）

## [0.3.0] - 2026-09-06
### Added
- **外部桌宠支持**：导入本地 zip/目录自定义宠物，或从 Petdex 在线仓库下载；管理面板支持导入/描述编辑/重命名/删除/一键热切换；manifest 结构、帧率、尺寸、语音清单全量校验，IPC 宠物 ID 路径逃逸防护；无语音宠物能力门控自动降级（动画照常），语音能力双向同步
- **WorkBuddy 第六工具支持**：心跳目录驱动的会话发现（严格 UUID 校验 + prewarm 池过滤，macOS/Windows 同一套语义）、红绿灯状态监控、Skill/MCP 资源管理接入、官方品牌图标
- **APP 类会话卡与跳转**：持久未读卡（转绿跨 MAM 重启保留、点击跳转/手动关闭后消失、宿主进程退出全部清理）；看板与桌宠卡片支持手动 X 关闭；会话级深度链接直达第一优先（`workbuddy://chat/<id>`、`codex://threads/<id>`），派发前 handler 校验 + 派发后前台验证，失败落 APP 级保底聚焦且不误标已读；Windows 深度链接支持（含 MSIX 协议注册形态判定）
- **工具勾选管理**（设置页新分区）：行式开关（图标 + 名称 + 安装状态 badge），批量保存 + 确认弹窗 + 未保存离开拦截；取消勾选将链接还原为真实文件、移除 MCP 条目并清空未读卡，SSOT 仓库与 DB 分配关系保留，重新勾选按原分配整体重建；未勾选工具彻底隐藏（扫描跳过/通知静音/界面不出现），写命令返回结构化错误码
- 资源表格按工具文件夹快捷入口、表头点击按名称排序
### Fixed
- 工具勾选管理打磨（issue #36 全 8 项）：命令层守卫全覆盖、保存异步化不再冻结窗口 IPC、守卫错误码 i18n（中英双语）、还原结果 kept/lost 分账分级提示、`~/.claude.json` 写回不再按键序重排、工具行图标、安装判定口径统一（目录 OR CLI）、先文件后数据库 + 部分失败自动回滚（重新勾选即幂等重试，无需先关再开）
- 会话监控稳定性（issue #35）：标题读取回退、兜底扫描缓存、pid 交叉校验、每轮 SQLite 连接复用；心跳观察持久化实现跨重启补偿；已读墓碑持久化，杜绝「已读复活」
- APP 跳转 P2 批次（issue #34）：冷启动深度链接链进程快照刷新、AppleScript bundle 字面量反斜杠转义、宿主进程 `is_host_process` 匹配、宠物窗口 toaster 挂载、候选聚焦失败兜底、历史跳转走 deep-link 优先
- WorkBuddy Windows 实测修订：盘符路径 mangle + JSONL 目录扫描兜底、MSIX 版 ChatGPT 宿主发现（Codex APP 聚合）、心跳新鲜度阈值校准 90s
- 修复设置页暗色/亮色主题切换从未生效的问题（Context 改事件驱动 store）
- 工具勾选变化跨窗口实时广播，主窗口缓存即时刷新
- 设置页未保存离开拦截与窗口关闭时序修复
- 桌宠：终端会话关闭立即清理对应卡片；失败音频探测止停；语音删除限定四组语音；活动宠物删除/切换后自动恢复
- Release 工作流：macOS 补建 `app` bundle，修复 updater 产物（`.app.tar.gz`）从未生成、`latest.json` 缺 darwin 平台导致 macOS 自动更新不生效的问题
### Changed
- 三处硬编码工具列表改为后端 `enabled_tools` 下发，看板双视图与设置页同源
- Codex APP 已读绿灯卡自动消失，池化宿主未读状态聚合展示
- MCP JSON 写回启用 `preserve_order`，工具配置文件不再被整体按字母序重排

## [0.2.0] - 2026-09-02
### Added
- Foxbell 桌宠：独立悬浮窗口（可置顶），状态卡片（红等待/黄运行/绿完成未读）+ 31 条状态语音 + 拖拽物理 + 完整右键菜单（大小三档/三场景动作绑定）；完成提示音接管与置顶时浮窗抑制；入口：看板设置/🦊 按钮/托盘/右键菜单（Tauri 暂无穿透 API，透明区遮挡下层点击）
- Kimi Code 第五工具支持：会话监控、Skill/MCP 管理、`KIMI_CODE_HOME` 数据目录重定向（自动回退旧版 `~/.kimi`）
- 资源管理增强：逐行批量启用/禁用、全类型搜索、扩展清单安装对话框与严格 semver 版本校验
### Fixed
- Kimi：修复轮次答完后状态灯一直卡黄（`turn.ended` 事件未映射）；`turn.ended` 现为唯一轮次结束信号，`usage.record` 不再映射以消除轮中进行中的瞬态误标
- Kimi：会话索引根目录一致性、越界索引项跳过、非 ASCII 会话标题截断 panic 等多项修复
- 修复 pull_request CI 因 `GITHUB_TOKEN` 缺少 `pull-requests: read` 权限而一直静默失败的问题
- 修复 Windows 终端跳转链（每进程 Codex 独立卡片、前台聚焦与逐层回退）
- 修复多工具上报同一会话时的重复卡片（按工具 + 会话 ID 聚合去重）
- macOS 启用 private API 修复透明窗口显示
### Changed
- monitor 解析器按工具拆分（claude/codex/kimi 等），抽出 cwd/JSONL/git 公共设施
- 适配器统一经中央工具注册表分发服务层调用
- Release 流程改为草稿优先：tag 触发云端编译后生成 draft release，人工验收后再发布

## [0.2.2] - 2026-07-08
### Added
- 资源管理看板双视图（按类型/按工具）
- 预设组一键应用/取消功能
- 兼容性检查对话框
- OpenClaw 第四工具支持
### Fixed
- 修复 TypeScript 未使用变量错误

## [0.1.0] - 2026-07-01
### Added
- 多 Agent 工具统一监控看板
- Skill/MCP/Plugin 三层映射架构
- 系统托盘预设菜单
- 状态变更桌面通知
- 终端快速跳转
