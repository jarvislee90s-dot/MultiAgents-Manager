# Changelog

## [Unreleased]
### Changed
- **会话扫描三层预算（性能）**：前端 3 秒轮询下各解析器不再每轮全量重扫历史会话（实机 codex 会话库 2GB / 388 文件曾把主线程打满 100%、界面卡死）。三层通用机制落在 `monitor/session_scan`：① 零进程零解析（`get_all_sessions` 编排层统一短路 + 全 adapter 防回归测试）；② `(mtime,size)` 内容摘要缓存——解析拆「纯内容产物（缓存）+ 时间叠加（现算）」，codex / claude / kimi / workbuddy 已接入；③ 无界历史扫描限 24h 新鲜窗口 + 活跃进程窗口内匹配不到才回退全量（codex；进程界定有界扫描不窗口化以免丢空闲超窗的活跃卡）。opencode / zcode（SQLite 查询即过滤）与 openclaw（配置界定）仅享 ①。契约已写入 AGENTS.md「Agent Adapter 模式」与 trait 文档，新工具接入自动受保护

## [0.4.0] - 2026-09-09
### Added
- **ZCode 第七工具支持**（智谱桌面 AI 编程助手，Electron APP 形态）：宿主判定（进程侧只回答「应用开没开」，Windows 全部可执行体同名 ZCode.exe 时按命令行区分主进程/辅助进程/会话运行时）+ 双 SQLite 会话聚合（tasks-index 任务索引 + session/message/part 消息流，每会话一卡、24h 窗口、archived/deleted 过滤、会话 id 严格 UUID 校验）；状态从消息流尾部按 sequence 倒扫推导（step-finish 懒落库时间戳不可靠、顺序可靠），todo_reminder 等记账消息识别跳过；子代理长任务经共享层「后代活跃度仲裁」保持运行中（主会话静默 + 子会话活跃 = 健康等待，停更且无后代活动 = 疑似卡住）；task_status=error 按完成转绿。资源管理：skill 分发（SSOT → `~/.zcode/skills/` 建链/删链，**目录已实测被 ZCode 识别**）+ MCP 读-改-写（`~/.zcode/cli/config.json` 仅 `mcp.servers` 子树，未知键与原键序保留，解析失败只报错不落盘，`enable:false` 如实展示为停用）
- **ZCode 跳转 = 直接聚焦唯一窗口**：ZCode 为单窗口多标签应用（实机取证：`windowId:1` 恒定、标题恒为 "ZCode" 不含工作区名），点击卡片经 pid 单窗口路径零歧义锁定，失败落 APP 级激活兜底。工作区深链 `zcode://workspace/open` 经实机验收后**有意不接入**：其语义为打开工作区 + 全新会话 composer（非定位已有会话）、每次派发无条件弹信任确认且拉起一个转发进程
- **同项目双开跳转直达（窗口标题匹配层）**：Kimi / OpenCode 的终端窗口标题与会话标题（kimi `state.json` 标题 / OpenCode DB 标题）归一化比对（剥 spinner 前缀 / "OC | " 工具前缀 / 尾部省略号），唯一命中即锁定——双开终端不再弹选择器；会话标题经全部跳转入口（看板 / 通知 / 桌宠 / 历史）统一透传
- 卡片项目名显示预算：中文封顶 10 字、英文封顶 15 字符（15 单位宽度预算，CJK 每字 1.5 单位），截断点确定、不随卡宽漂移，完整名悬停可见
- 共享层通用化（D5）：停更降级新增后代活跃度仲裁参数（空后代语义 = 现状，既有工具行为零变化并有逐边界回归测试）；数据驱动持久绿卡门（P1-3 剔除 + 未读态标记）从 Codex 特判泛化为工具能力判定，ZCode 聚合卡复用同款语义

### Fixed
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
