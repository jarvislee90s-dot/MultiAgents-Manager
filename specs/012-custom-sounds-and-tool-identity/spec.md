# 功能规格说明：自定义提示音与工具标识

**功能分支**：`012-custom-sounds-and-tool-identity`

**创建日期**：2026-08-25

**状态**：草稿

**输入**：用户新增需求（已确认决策：仅配置任务完成音、彻底替换现有 Hz 合成音、颜色先行 + 品牌图标）。素材勘察：`assets/NoticeSound/` 含 12 个通用音效 WAV（共约 4.8MB，成功×4/失败×2/消息×2/搜索/激活/激活完成/错误），不在前端构建范围；品牌图标三个已拉取入库 `assets/icons/`（claude.svg、openai.svg 来自 simple-icons，opencode.svg 来自 opencode.ai 官网，均规整为可缩放 SVG）。

## 用户场景与测试

### 用户故事 1 — 按工具配置任务完成提示音（优先级: P1）

**验收场景**：

1. **给定** 设置页提示音区，**当** 查看，**则** 有"全局默认完成音"+ 四个工具（Claude/Codex/OpenCode/OpenClaw）各自的完成音选择，每个选择可为音效库任一音、或"静音"，未单独设置的工具跟随全局默认
2. **给定** 每个音效选项旁有试听按钮，**当** 点击，**则** 立即播放该音效
3. **给定** claude 配置了专属完成音、codex 未配置，**当** 两工具任务先后结束（黄→绿），**则** 分别播放 claude 专属音与全局默认音
4. **给定** 某工具完成音设为静音，**当** 其任务结束，**则** 无声但弹窗照常
5. **给定** 任务开始（绿→红/黄），**则** 不播放任何提示音（仅黄→绿触发，声音方向过滤显式实现，不依赖巧合）

### 用户故事 2 — 合成音彻底移除（优先级: P2）

**验收场景**：

1. **给定** 设置页，**当** 查看，**则** Hz 频率调节 UI 不存在；`src/lib/audio.ts` 的合成音代码（playTone/频率配置/保存）被文件音效实现替换
2. **给定** 升级用户本地存有旧的 `mam-audio-frequencies` localStorage 键，**则** 被忽略或清理，不报错

### 用户故事 3 — 卡片与通知窗的工具标识统一（优先级: P1）

**验收场景**：

1. **给定** 首页会话卡片，**当** 查看，**则** 工具标识从左到右为：品牌图标（SVG）→ 工具名（带工具色）→ 项目文件夹名 → session 前 8 位标识；配色为 codex（App 与 CLI 同色）紫、claude 橙、opencode 灰底白字、openclaw 灰（新增徽标，补齐缺失条目）
2. **给定** 通知浮窗，**当** 弹出，**则** 显示与卡片一致的品牌图标与工具色（当前仅文字）
3. **给定** 明暗主题切换，**则** 图标随文字颜色适配（currentColor），无"看不见"的配色组合
4. **给定** 中英文界面，**则** 工具名与相关文案跟随 i18n（键集对齐，check:i18n 门禁通过）

## 设计

### 1. 音效系统（替换 `src/lib/audio.ts`）

- **素材入构建**：`assets/NoticeSound/*.wav` 拷贝至 `public/sounds/`（前端构建打包；`assets/` 目录不在构建范围）。12 个文件名作为音效 ID
- **播放引擎**：`AudioContext` + `fetch('/sounds/<id>.wav')` → `decodeAudioData`（首播解码后按 id 缓存 `AudioBuffer`）→ `AudioBufferSourceNode` 播放。保留现有 suspended→resume 处理
- **配置存储**（localStorage `mam-sound-config`）：`{ default: <id|'mute'>, tools: { claude?: <id|'mute'>, codex?: …, opencode?: …, openclaw?: … } }`
- **触发规则**：仅当颜色变化目标为绿（黄/红→绿）时播放：`tools[agentType] ?? default`，值为 `mute` 则跳过。`playSoundForStatus` 及 waiting 双音、Hz 相关代码全部删除
- **设置页**：提示音区重做——全局默认下拉（12 音效 + 静音 + 试听）、四工具行（"跟随全局"为默认态 + 可选专属/静音 + 试听）。沿用设置页现有组件与 i18n 模式

### 2. 工具标识（SessionCard / notification.tsx / 图标）

- **图标**：`assets/icons/{claude,openai,opencode}.svg` 移入 `src/assets/icons/`，组件内引用渲染（`currentColor` 继承文字色，claude/openai 天然适配双主题；opencode 官网图标自带配色）。Codex（App+CLI）共用 openai 图标；openclaw 暂无品牌图标，用文字徽标（灰），后续有素材再补
- **配色**（`AGENT_BADGE` 更新 + openclaw 补条目）：codex→紫（`purple-*`）、claude→橙（`orange-*`）、opencode→灰底白字（`zinc-*`）、openclaw→灰。状态灯红黄绿体系不动
- **卡片标题布局**：调整为 `[图标] [工具名(色)] [文件夹名] [session 8 位]` 从左到右；通知窗头部同样加图标+工具色工具名
- 与 i18n 键集对齐（新增键 zh/en 同步）

## 范围外

- 等待音（红）的任何声音配置（用户决策：仅完成音）
- 音频文件压缩转码（4.8MB WAV 桌面应用可接受；如需减重另行处理）
- openclaw 品牌图标（无现成素材，文字徽标占位）
- WebAudio autoplay 策略的深度防御（若修复映射后仍偶发无声再查，现有 resume 处理保留）

## 测试策略

- 纯逻辑单测（如可行）：配置解析与默认值合并（`tools[x] ?? default`、mute 语义）
- `pnpm check:i18n` 键对齐；`pnpm lint`/`build`
- 人工验证：三个用户故事全部场景（试听、按工具区分播放、静音、开始任务不响、双主题图标、卡片/通知窗一致性）
