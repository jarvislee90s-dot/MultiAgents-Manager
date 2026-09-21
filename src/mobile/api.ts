// 移动端 API：纯 fetch，零 Tauri 依赖（PWA/浏览器同构）
import type { SessionsResponse, TransitionEvent } from "@/types/session";

/** 带 HTTP 状态的请求失败（status=null 表示网络层异常，无响应可读）。
 *  详情页/预览按 status 分流错误文案（404 → 会话不可读，403 → 文件不可预览）；
 *  data 携带错误响应体 JSON（/pair/pin 的 error/remaining/retryAfter），供 PairPage 分診文案 */
export class ApiError extends Error {
  status: number | null;
  data: Record<string, unknown> | null;
  constructor(status: number | null, message: string, data: Record<string, unknown> | null = null) {
    super(message);
    this.status = status;
    this.data = data;
  }
}

/** 访问密码配对（M5 A7，唯一配对入口）：POST /pair/pin。
 *  成功 → { ok: true }（180 天 cookie 已由响应 Set-Cookie 落地）；
 *  失败 → 抛 ApiError：HTTP 状态在 status、响应体 JSON（error/remaining/retryAfter）
 *  在 data——PairPage 据此分診「剩余次数 / 锁定 / 未设密码 / 设备满 / 网络」文案。
 *  429（限速锁定）与 403（设备上限）也走 ApiError（不再像旧 /pair 静默吞掉） */
export async function pairWithPin(pin: string): Promise<{ ok: boolean }> {
  let r: Response;
  try {
    r = await fetch("/m/api/v1/pair/pin", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ pin }),
    });
  } catch (e) {
    throw new ApiError(null, `pair/pin 网络异常: ${String(e)}`);
  }
  if (!r.ok) {
    let data: Record<string, unknown> | null = null;
    try {
      data = (await r.json()) as Record<string, unknown>;
    } catch {
      /* 错误体非 JSON（代理注入页等）：data 保持 null，上层按状态码兜底文案 */
    }
    throw new ApiError(r.status, `pair/pin ${r.status}`, data);
  }
  return { ok: true };
}

export async function fetchSessions<T>(): Promise<T | null> {
  const r = await fetch("/m/api/v1/sessions");
  if (r.status === 403) return null; // 设备失效 → 回配对页
  return r.json() as Promise<T>;
}

// host 载荷（GET /m/api/v1/host）：host 部分对应 Rust host_payload 的 "host" 键；
// enabledTools 为 P8d 受管工具 id 列表（Task 3 chips 过滤的数据源）；
// bootId 为 MAM 进程生命周期标识（书签等「随进程消失」的客户端态的恢复守卫）
export interface HostInfo {
  name: string;
  platform: "macos" | "windows" | "linux";
  version: string;
  bootId: string;
}

export interface HostPayload {
  host: HostInfo;
  enabledTools: string[];
}

export async function fetchHost<T = HostPayload>(): Promise<T | null> {
  const r = await fetch("/m/api/v1/host");
  if (r.status === 403) return null; // 设备失效 → 与 sessions 同语义
  return r.json() as Promise<T>;
}

// ==== M3 Task 8：会话详情（ZCode 式对话视图）+ 文件预览 ====

/** 统一消息条目 — 与 Rust `remote::content::SessionMessage`（camelCase 序列化）逐字段
 *  对应，勿漂移：seq / role / content / kind / ts / toolName? / toolArgs? / collapsed。
 *  kind ∈ user / assistant / thinking / tool-call / tool-result / plan；
 *  thinking 与 tool-call 的 collapsed 恒 true（wire 语义，运行中态的默认折叠依据）；
 *  plan 是 T1 升格的一等计划消息（content = 计划 markdown 原文，collapsed 恒 false，
 *  toolName 保留供辨识、toolArgs 恒空——不透传参数串） */
export interface SessionMessage {
  seq: number;
  role: string;
  content: string;
  kind: "user" | "assistant" | "thinking" | "tool-call" | "tool-result" | "plan" | string;
  ts: number | null;
  toolName?: string | null;
  toolArgs?: string | null;
  collapsed: boolean;
}

/** 会话消息页（Bug 1，M3 验收）：messages + truncated（文件头部被字节窗截断的
 *  标记——SQLite 系与 dsh 恒 false）。truncated=true 表示存在更早未在本页的内容，
 *  即使条数 < limit 也应显示「加载更早消息」 */
export interface SessionMessagesPage {
  messages: SessionMessage[];
  truncated: boolean;
}

/** 拉取单会话消息流尾部（八工具统一出口）。读取失败（会话不存在 / 存储不可读）
 *  以 ApiError 抛出：404 = 会话内容不可读；网络异常 status=null。
 *  本层无状态：SSE transition 不驱动详情页（M3 范围裁决不变），10s 轮询节奏由
 *  SessionDetail 页面层驱动（F6），此处只负责单次拉取 */
export async function fetchSessionMessages(
  agentType: string,
  sessionId: string,
  limit: number
): Promise<SessionMessagesPage> {
  const q = new URLSearchParams({
    agent_type: agentType,
    session_id: sessionId,
    limit: String(limit),
  });
  let r: Response;
  try {
    r = await fetch(`/m/api/v1/session-messages?${q}`);
  } catch (e) {
    throw new ApiError(null, `session-messages 网络异常: ${String(e)}`);
  }
  if (!r.ok) throw new ApiError(r.status, `session-messages ${r.status}`);
  const j = (await r.json()) as { messages?: SessionMessage[]; truncated?: boolean };
  return {
    messages: Array.isArray(j.messages) ? j.messages : [],
    truncated: j.truncated === true,
  };
}

/** 文件条目（M3+ 文件面板，与 Rust `remote::files::FileEntry` camelCase 序列化
 *  逐字段对应，勿漂移）：path = 最后一次出现的原始形态，lastSeq = 最后出现条目的
 *  会话内序，lastTs = 最后出现时间（可 null → 前端显示 `—`），hits = 出现次数
 *  （后端契约字段；2026-09-16 用户裁决：次数信息用户不在意，前端**不再展示**，
 *  保留字段供后续可能的排序/统计消费），
 *  modified = 是否被写类工具（Edit/Write…）改写触达（2026-09-16 用户裁决：
 *  前端据此把「仅读过」的文件名渲成常规色，与「动过手」的区分），
 *  origin = 来源三池（M5 B2）：user=我上传的 / tool_read=工具读取 / tool_write=
 *  工具读写；旧载荷无此键 → undefined（来源筛选按「全部来源」处理，行上不渲染徽标） */
export interface SessionFileEntry {
  path: string;
  lastSeq: number;
  lastTs: number | null;
  hits: number;
  modified: boolean;
  origin?: "user" | "tool_read" | "tool_write";
}

/** 拉取该会话涉及的文件表（/session-files，泛化提取）。一份数据两用：正文
 *  路径链接化（取 path 集）+ 文件面板列表（全字段）。增强能力：任何失败
 *  （含 404/403）静默降级为空表——详情页正文照常渲染。
 *  limit = 追溯档位（面板 200/500/1000 三档），透传后端窗口机制；
 *  truncated = 该档位下还有更早文件未纳入（面板据此提示） */
export async function fetchSessionFiles(
  agentType: string,
  sessionId: string,
  limit: number
): Promise<{ files: SessionFileEntry[]; truncated: boolean }> {
  const q = new URLSearchParams({
    agent_type: agentType,
    session_id: sessionId,
    limit: String(limit),
  });
  try {
    const r = await fetch(`/m/api/v1/session-files?${q}`);
    if (!r.ok) return { files: [], truncated: false };
    const j = (await r.json()) as { files?: SessionFileEntry[]; truncated?: boolean };
    return {
      files: Array.isArray(j.files) ? j.files : [],
      truncated: j.truncated === true,
    };
  } catch {
    return { files: [], truncated: false };
  }
}

/** 文件预览载荷：图片 → 同源 fetch blob 后的 object URL（调用方负责 revoke）；
 *  其余 → 文本内容 + 后端判定的 mime */
export type FilePayload =
  { kind: "image"; url: string; mime: string } | { kind: "text"; content: string; mime: string };

/** 安全读取会话项目目录内的文件（/file）。越界 / 超限 / 不存在对外一律 403
 *  （后端探测面最小化，不可区分）；session_id 不在快照 → 404；网络异常 → status=null */
export async function fetchFile(sessionId: string, filePath: string): Promise<FilePayload> {
  const q = new URLSearchParams({ session_id: sessionId, path: filePath });
  let r: Response;
  try {
    r = await fetch(`/m/api/v1/file?${q}`);
  } catch (e) {
    throw new ApiError(null, `file 网络异常: ${String(e)}`);
  }
  if (!r.ok) {
    // M5 P2-a：403 响应体带结构化原因码（sensitive/too_large/not_found/not_file/io），
    // FilePreview 据此分診排障文案
    let data: Record<string, unknown> | null = null;
    try {
      data = (await r.json()) as Record<string, unknown>;
    } catch {
      /* 非 JSON 错误体（代理页等）：data 保持 null，按状态码兜底文案 */
    }
    throw new ApiError(r.status, `file ${r.status}`, data);
  }
  const mime = r.headers.get("content-type")?.split(";")[0]?.trim() ?? "";
  if (mime.startsWith("image/")) {
    const blob = await r.blob();
    return { kind: "image", url: URL.createObjectURL(blob), mime };
  }
  const j = (await r.json()) as { content: string; mime: string };
  return { kind: "text", content: j.content, mime: j.mime };
}

// ==== M3 Task 6：SSE 实时通道客户端 ====

/** SSE 端点路径（唯一来源：EventSource 与测试 mock 都引用它） */
export const EVENTS_PATH = "/m/api/v1/events";

/** 断流后重连的退避基数（毫秒）：第 N 次失败等待 N×基准，N 达降级阈值即转轮询 */
export const RECONNECT_BASE_MS = 1000;

/** 降级阈值：连续失败达到该次数即放弃 SSE、转轮询
 *  （C1 验收「断 SSE → 2 次失败 → 3s 轮询」） */
export const DEGRADE_AFTER_FAILURES = 2;

/**
 * 订阅服务端事件流（GET /m/api/v1/events），返回停止函数。
 *
 * 两条数据通道：
 * - `onSnapshot(SessionsResponse)`：连接建立后的首帧全量快照（也是断线重连后的基线校正）；
 * - `onTransition(TransitionEvent)`：watcher 已去重的状态跃迁边沿（**铁律 4：本层不独立
 *   去重**，来一条转一条）。
 *
 * `onDegraded()`：连续失败达阈值后的**一次性**信号——SSE 通道放弃，由调用方切换轮询
 * （本函数**不接管轮询**：降级后的 3s 轮询复用调用方既有的 tick 逻辑，避免两套轮询
 * 并存；简报骨架里 connectEvents 自带轮询循环，与「复用现有 tick」的控制者裁决冲突，
 * 裁决优先——轮询留在 Board，in-flight 守卫 / 403→onUnpaired / 错误横幅语义单点保留）。
 *
 * 断线策略（控制者裁决，勿改成"依赖浏览器内建重连"）：`onerror` 里显式 `close()` +
 * 手动线性退避重连（N×1s），而非放任 EventSource 自愈——因为需要**可数的失败语义**
 * 才能实现「2 次失败降级轮询」；浏览器内建重连（约 3s 固定间隔）不可观测、与手动重连
 * 竞争，故关闭内建、自管节奏。
 *
 * 403 语义（与 fetchSessions 口径一致）：EventSource 拿不到 HTTP 状态码，403（设备失效）
 * 与网络断流对 `onerror` 不可区分——**这里一律不判设备失效**（SSE 断线不等于 403，
 * 服务器重启/网络抖动都会断流，误踢回配对页会打断用户）。设备失效判定唯一走
 * fetchSessions 返回 null 的通道（初始探测与降级轮询）。
 *
 * 不做「轮询期间定期试回 SSE」（YAGNI，M3 不要求）：服务端恢复后刷新页面即重连；
 * 常驻双通道探测会引入额外连接与状态机复杂度，收益不成比例。
 */
export function connectEvents(
  onSnapshot: (data: SessionsResponse) => void,
  onTransition: (data: TransitionEvent) => void,
  onDegraded: () => void
): () => void {
  let es: EventSource | null = null;
  let failures = 0;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let degraded = false;
  let stopped = false; // 停止后禁止任何重连再被装上

  function degrade() {
    if (degraded) return;
    degraded = true;
    es?.close();
    es = null;
    onDegraded();
  }

  /** 解析帧并转交回调；坏帧（截断 / 代理注入）跳过该帧，连接保持——
   *  不得因单帧 JSON 错误断流 */
  function dispatch<T>(e: Event, handler: (data: T) => void) {
    try {
      const parsed = JSON.parse((e as MessageEvent).data) as T;
      // 成功解析出一帧即清零：退避计数只反映"连续失败"（收到合法帧 = 链路健康）
      failures = 0;
      handler(parsed);
    } catch {
      /* 坏帧：忽略（不清零计数——能收到字节但解析不了不足以证明链路健康） */
    }
  }

  function connect() {
    if (stopped || degraded) return;
    let next: EventSource;
    try {
      next = new EventSource(EVENTS_PATH);
    } catch {
      // 环境不支持 EventSource（老浏览器 / 无该全局的运行时）：不做无谓退避重试，
      // 直接降级轮询——与「连不上即降级」同一终局，省掉注定失败的两次握手
      degrade();
      return;
    }
    es = next;
    next.addEventListener("snapshot", (e) => dispatch(e, onSnapshot));
    next.addEventListener("transition", (e) => dispatch(e, onTransition));
    next.onerror = () => {
      if (stopped || degraded) return;
      next.close(); // 关掉内建重连（见函数注释的断线策略）
      if (es === next) es = null;
      failures += 1;
      if (failures >= DEGRADE_AFTER_FAILURES) {
        degrade();
      } else {
        reconnectTimer = setTimeout(connect, RECONNECT_BASE_MS * failures); // 线性退避
      }
    };
  }

  connect();
  return () => {
    stopped = true;
    es?.close();
    es = null;
    if (reconnectTimer) clearTimeout(reconnectTimer);
  };
}

// ==== M7 Task 7：注入发送（W4 移动端发送 UI）====

/** 输入区可用性矩阵（GET /session-send-info 载荷，与 Rust `session_send_info`
 *  的 JSON 逐字段对应，勿漂移）：injectable=false 时 reasonCode/reason 携带不可
 *  注入原因（如 WorkBuddy 黑盒），**channels/visibility 不返回**（后端
 *  RouteOutcome::NotInjectable 分支只给 {injectable,reasonCode,reason}）→ 前端
 *  类型须 optional（M9R P2-10 对齐）；injectable=true 时 channels 为候选注入
 *  通道（tmux/iterm2/…），visibility=after_refresh 表示注入后需刷新才见回显 */
export interface SendInfo {
  injectable: boolean;
  reasonCode?: string;
  reason?: string;
  channels?: string[];
  visibility?: "realtime" | "after_refresh";
}

/** 拉取输入区可用性（W4：输入区挂载时一次）。403（设备失效，与 fetchSessions
 *  同语义）→ null；其余失败（404 会话不在快照 / 网络异常）→ 抛 ApiError，
 *  由调用方静默降级（不渲染输入区，详情页正文照常） */
export async function fetchSendInfo(sessionId: string): Promise<SendInfo | null> {
  const q = new URLSearchParams({ session_id: sessionId });
  let r: Response;
  try {
    r = await fetch(`/m/api/v1/session-send-info?${q}`);
  } catch (e) {
    throw new ApiError(null, `session-send-info 网络异常: ${String(e)}`);
  }
  if (r.status === 403) return null; // 设备失效 → 回配对页
  if (!r.ok) throw new ApiError(r.status, `session-send-info ${r.status}`);
  return (await r.json()) as SendInfo;
}

/** 发送回执（POST /session-send 响应四态，HTTP 200 恒定，语义在 body.status）：
 *  delivered=已直送终端；submitted=已投递未确认（D7/T3 确认判据收紧，验收问题 #5：
 *  注入 Ok + 戳未中 + 屏读无滞留草稿 = 消息已被 TUI 收进内部队列，agent 空闲后
 *  处理——中性态，非失败、**不提供重试**，重试 = 双发且 TUI 那份无法撤回）；
 *  queued=运行中留队（itemId+position 供插队/撤回/排队指示）；failed=注入失败回执
 *  （error 文案可直接展示；失败行已退出 pending，队列无残留，重按发送即重试） */
export type SendResult =
  | { status: "delivered" }
  | { status: "submitted" }
  | { status: "queued"; itemId: number; position: number }
  | { status: "failed"; error: string };

/** 发送消息（W4 直发/入队分派，后端按输入态路由；多行原样上行，归一在服务端
 *  入队时一次完成）。非 2xx（400 参数非法 / 404 会话消失 / 403 不可注入）→
 *  抛 ApiError。
 *  queueOnly（D6 修改重发，可选）：true = 只入队（后端跳过直发尝试，即使快照显示
 *  可输入也强制入队，防变相插队）；入队后 flush 循环对其与普通队列项同权（转闲
 *  按序自动放行）。**缺省不带该键**——保持既有请求体形态不变（普通发送路径零漂移） */
export async function sessionSend(
  sessionId: string,
  text: string,
  queueOnly?: boolean
): Promise<SendResult> {
  let r: Response;
  try {
    r = await fetch("/m/api/v1/session-send", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ sessionId, text, ...(queueOnly ? { queueOnly: true } : {}) }),
    });
  } catch (e) {
    throw new ApiError(null, `session-send 网络异常: ${String(e)}`);
  }
  if (!r.ok) {
    // 403 not_injectable{reason,reasonCode}（如挂载后会话漂移为 APP/黑盒形态）——
    // 后端已备好中文 reason，解析进 data 供调用方展示（对齐 fetchFile 惯例）
    let data: Record<string, unknown> | null = null;
    try {
      data = (await r.json()) as Record<string, unknown>;
    } catch {
      /* 交验失败保持 null */
    }
    throw new ApiError(r.status, `session-send ${r.status}`, data);
  }
  return (await r.json()) as SendResult;
}

/** 上传附件（2026-09-20）：原始字节 POST 到 /session-attachment——服务端落盘到
 *  **用户项目目录** .mam-attachments/<会话>/，返回绝对路径供消息内联标记
 *  （<image|file path>，文件池既有约定）引用。
 *  错误契约：403 → null（设备失效，与 fetchSendInfo 同口径）；404 →
 *  ApiError(404, "no_session"|"no_cwd")（composer 据后者禁用上传钮）；
 *  413 → ApiError(413, "too_large")；其余非 2xx → ApiError(status) */
export async function uploadAttachment(
  sessionId: string,
  file: File,
  signal?: AbortSignal
): Promise<{ path: string; size: number } | null> {
  const q = new URLSearchParams({ session_id: sessionId, name: file.name });
  let r: Response;
  try {
    r = await fetch(`/m/api/v1/session-attachment?${q}`, {
      method: "POST",
      headers: { "content-type": "application/octet-stream" },
      body: await file.arrayBuffer(),
      signal,
    });
  } catch (e) {
    throw new ApiError(null, `session-attachment 网络异常: ${String(e)}`);
  }
  if (r.status === 403) return null; // 设备失效 → 回配对页（fetchSendInfo 同口径）
  if (!r.ok) {
    let reason = `session-attachment ${r.status}`;
    try {
      const j = (await r.json()) as { error?: unknown };
      if (typeof j?.error === "string") reason = j.error;
    } catch {
      /* 响应体非 JSON：保留默认 reason */
    }
    throw new ApiError(r.status, reason);
  }
  return (await r.json()) as { path: string; size: number };
}

/** 排队条目视图（GET /session-queue 的 items 元素，camelCase 契约）：position =
 *  1 起队位；content 为入队时 compose 完成的最终注入文本（含设备名前缀） */
export interface QueueItemView {
  id: number;
  content: string;
  enqueuedAt: number;
  position: number;
}

/** 拉取该会话待发队列（FIFO，W4 排队指示/轮询刷新的数据源）。非 2xx → 抛 ApiError */
export async function fetchQueue(sessionId: string): Promise<QueueItemView[]> {
  const q = new URLSearchParams({ session_id: sessionId });
  let r: Response;
  try {
    r = await fetch(`/m/api/v1/session-queue?${q}`);
  } catch (e) {
    throw new ApiError(null, `session-queue 网络异常: ${String(e)}`);
  }
  if (!r.ok) throw new ApiError(r.status, `session-queue ${r.status}`);
  const j = (await r.json()) as { items?: QueueItemView[] };
  return Array.isArray(j.items) ? j.items : [];
}

/** 插队直发（裁决 12）：按 itemId 点名该会话 pending 中的一条即刻注入。
 *  回执五态精确映射（F7④ + D7/T3 与后端契约对齐）：Sent → delivered；Failed(e) →
 *  failed{error}（注入失败行已退出 pending，可重发）；submitted → submitted
 *  （防御性契约对齐：Submitted 仅直发分诊产出，插队以占用排空定论、后端本臂实际
 *  不可达——前端按非 delivered 走对账收敛即可）；Deferred | Suspended →
 *  queued{itemId,position}（行保持 pending，语义即排队）；守卫忙（该会话
 *  in-flight 投递占用）→ queued{itemId,position}（F1 新语义：旧忙时回 failed
 *  逼客户端重试，现改 queued 让位给进行中的投递）。非 2xx（404 not_found
 *  条目已不在队）→ 抛 ApiError */
export type QueueJumpResult =
  | { status: "delivered" }
  | { status: "submitted" }
  | { status: "queued"; itemId: number; position: number }
  | { status: "failed"; error: string };

export async function queueJump(sessionId: string, itemId: number): Promise<QueueJumpResult> {
  let r: Response;
  try {
    r = await fetch("/m/api/v1/session-queue/jump", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ sessionId, itemId }),
    });
  } catch (e) {
    throw new ApiError(null, `session-queue/jump 网络异常: ${String(e)}`);
  }
  if (!r.ok) throw new ApiError(r.status, `session-queue/jump ${r.status}`);
  return (await r.json()) as QueueJumpResult;
}

/** 撤回排队条目（W4）：200 {ok:true} 撤回成功；P2-6 忙时（该会话投递进行中）
 *  → 200 {status:"failed",error:后端中文文案}（条目**未被撤**、仍在队——前端不
 *  消费该文案，以 fetchQueue 复核结果为准）；条目已不在队（已送达 / 他端撤回）
 *  → 404 not_found → 抛 ApiError（调用方按「已不在队列」收敛，不作失败提示） */
export async function queueRetract(
  sessionId: string,
  itemId: number
): Promise<{ ok: true } | { status: "failed"; error: string }> {
  let r: Response;
  try {
    r = await fetch("/m/api/v1/session-queue/retract", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ sessionId, itemId }),
    });
  } catch (e) {
    throw new ApiError(null, `session-queue/retract 网络异常: ${String(e)}`);
  }
  if (!r.ok) throw new ApiError(r.status, `session-queue/retract ${r.status}`);
  return (await r.json()) as { ok: true } | { status: "failed"; error: string };
}

// ==== M8 Task 12：审批选项卡（红卡一键应答，W6）====

/** 审批选项视图（GET /session-approve-options 载荷，与 Rust `session_approve_options`
 *  的 JSON 逐字段对应，勿漂移）：available=false（会话非 Waiting / 工具无映射 /
 *  提示未命中）时 options 恒空——移动端据此不渲染审批卡；options 只含 id+label，
 *  **键位不外泄给 UI**（投递层机密）；verifiedWith = 映射实测版本，currentVersion =
 *  CLI 探测版本（探测失败为 null），drift=true 时红卡提示降级路径（普通发送）；
 *  reason = 严格档降级原因（Task 10 下发，如「键位待实测确认，请用普通发送」）：
 *  available=false 且 reason 存在 → 卡片只渲染提示条不渲染按键（M9R 消费）；
 *  旧分支（非 Waiting / 无映射 / 未命中）不给该键 → optional */
export interface ApproveOptionsView {
  available: boolean;
  options: { id: string; label: string }[];
  verifiedWith: string;
  currentVersion: string | null;
  drift: boolean;
  reason?: string;
  /** 批次丙 T8：审批点 plan 聚合——计划确认类审批卡主体带计划全文（claude/codex
   *  的 kind="plan" 消息）或计划文件路径（kimi 的 kind="plan-file"，isFile=true
   *  → 前端走文件预览读全文）。null/缺省 = 无计划在场（只渲染选项） */
  plan?: { content: string; isFile: boolean } | null;
  /** 批次丙 R1-3：**降级警示**——命中审批但未读到终端对话框选项（终端可能正显示
   *  多选项，二元键可能错位）。前端在二元卡渲染脚注。null/缺省 = 未降级 */
  degradedHint?: string | null;
  /** 批次丙 T5：选项来自**对话框屏读**——id 形如 `dialog:<n>`，label 是屏上原文
   *  （如 "1. Yes, and use auto mode"）；前端据此渲染编号按钮组（点按注入数字键 n）。
   *  缺省/ false → 映射表二元项（既有渲染，前向兼容旧后端） */
  dialog?: boolean;
}

/** 拉取审批选项卡数据源（红卡挂载时一次）。非 2xx → 抛 ApiError（调用方静默
 *  降级不渲染，与 fetchSendInfo 失败静默同惯例） */
export async function fetchApproveOptions(sessionId: string): Promise<ApproveOptionsView> {
  const q = new URLSearchParams({ session_id: sessionId });
  let r: Response;
  try {
    r = await fetch(`/m/api/v1/session-approve-options?${q}`);
  } catch (e) {
    throw new ApiError(null, `session-approve-options 网络异常: ${String(e)}`);
  }
  if (!r.ok) throw new ApiError(r.status, `session-approve-options ${r.status}`);
  return (await r.json()) as ApproveOptionsView;
}

/** 审批应答回执（POST /session-approve 响应，HTTP 200 恒定，语义在 body.status）：
 *  key_sent=按键已投递终端；failed=投递失败 / in-flight 忙让位（error 为后端中文
 *  文案，如「该会话投递进行中，请稍后重试」，可重试） */
export type ApproveResult = { status: "key_sent" } | { status: "failed"; error: string };

/** 审批一键应答（M8 红卡）。200 {status:"key_sent"} | 200 {status:"failed",error}；
 *  409 {error:"not_waiting"} | 404 {error:"no_mapping"|"no_session"} → 非 2xx 抛
 *  ApiError（错误码解析进 data.error，调用方分診中文文案——not_waiting 已不在
 *  等待、no_mapping 降级走普通发送） */
export async function sessionApprove(sessionId: string, optionId: string): Promise<ApproveResult> {
  let r: Response;
  try {
    r = await fetch("/m/api/v1/session-approve", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ sessionId, optionId }),
    });
  } catch (e) {
    throw new ApiError(null, `session-approve 网络异常: ${String(e)}`);
  }
  if (!r.ok) {
    // 409/404 错误码在响应体 data.error——解析进 data 供调用方分診（对齐 sessionSend 惯例）
    let data: Record<string, unknown> | null = null;
    try {
      data = (await r.json()) as Record<string, unknown>;
    } catch {
      /* 非 JSON 错误体（代理注入页等）：data 保持 null，按 message 兜底 */
    }
    throw new ApiError(r.status, `session-approve ${r.status}`, data);
  }
  return (await r.json()) as ApproveResult;
}

// ==== 批次乙 T8：问答卡（AskUserQuestion，claude 先行）====

/** 问答选项视图（questions[].options[] 条目）：label + description——**编号是渲染层
 *  按 index 生成**，键位/数字不在此列（投递层细节不外泄 UI，approve 同纪律）；
 *  description 恒在（后端 json! 无条件输出，缺省解析为空串）→ 必填 string */
export interface QuestionOptionView {
  label: string;
  description: string;
}

/** 问答题目视图（GET /session-question 载荷 questions[] 条目，与 Rust
 *  `session_question` 的 JSON 逐字段对应，勿漂移）：multiSelect=false → 单选，
 *  点选项=直接提交；true → 多选，点选=勾选切换 + 「提交」钮。questions.length>1
 *  → 前端按只读卡渲染（多问题翻页键序未测，不做注入） */
export interface QuestionView {
  header?: string;
  question: string;
  multiSelect: boolean;
  options: QuestionOptionView[];
}

/** 问答可用性视图（GET /session-question 载荷）：available=false（双通道均未命中 /
 *  审批标记隔离 / 会话不在快照）→ questions 恒空——移动端据此不渲染问答卡；
 *  answerable=false（T3：该工具的问答键序未实测，如 codex）→ 渲染**只读卡** +
 *  引导终端作答，不显示可点选项（「未验不出键」）；source = 识别通道诊断
 *  （"mark"=hook 标记〔通道 A〕/"scan"=会话消息兜底〔通道 B〕） */
export interface QuestionInfoView {
  available: boolean;
  /** 可选：旧后端不带该字段时按 true 处理（前向兼容——只有明确 false 才降只读） */
  answerable?: boolean;
  questions: QuestionView[];
  source?: "mark" | "scan" | null;
}

/** 拉取问答卡数据源（卡片挂载时一次）。非 2xx → 抛 ApiError（调用方静默降级
 *  不渲染，fetchApproveOptions 失败静默同惯例） */
export async function fetchSessionQuestion(sessionId: string): Promise<QuestionInfoView> {
  const q = new URLSearchParams({ session_id: sessionId });
  let r: Response;
  try {
    r = await fetch(`/m/api/v1/session-question?${q}`);
  } catch (e) {
    throw new ApiError(null, `session-question 网络异常: ${String(e)}`);
  }
  if (!r.ok) throw new ApiError(r.status, `session-question ${r.status}`);
  return (await r.json()) as QuestionInfoView;
}

/** 问答应答动作：select=单选点选项（数字直接提交）；toggle=多选勾选切换；
 *  submit=多选三段式提交；cancel=取消问题（Esc） */
export type QuestionAnswerAction = "select" | "toggle" | "submit" | "cancel";

/** 问答应答回执（POST /session-question/answer 响应，HTTP 200 恒定，语义在
 *  body.status）：key_sent=按键序列已投递终端；failed=投递失败 / in-flight 忙让位
 *  （error 为后端中文文案，可重试） */
export type QuestionAnswerResult = { status: "key_sent" } | { status: "failed"; error: string };

/** 问答一键应答（T8）。index = 选项序号（0 起；select/toggle 必填）。
 *  409 {error:"no_question"|"multi_questions"} | 400 {error:"bad_request"|"bad_index"}
 *  → 非 2xx 抛 ApiError（错误码解析进 data.error，调用方分診中文文案） */
export async function sessionQuestionAnswer(
  sessionId: string,
  action: QuestionAnswerAction,
  index?: number
): Promise<QuestionAnswerResult> {
  let r: Response;
  try {
    r = await fetch("/m/api/v1/session-question/answer", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ sessionId, action, index }),
    });
  } catch (e) {
    throw new ApiError(null, `session-question/answer 网络异常: ${String(e)}`);
  }
  if (!r.ok) {
    // 409/400 错误码在响应体 data.error——解析进 data 供调用方分診（sessionApprove 惯例）
    let data: Record<string, unknown> | null = null;
    try {
      data = (await r.json()) as Record<string, unknown>;
    } catch {
      /* 非 JSON 错误体：data 保持 null，按 message 兜底 */
    }
    throw new ApiError(r.status, `session-question/answer ${r.status}`, data);
  }
  return (await r.json()) as QuestionAnswerResult;
}

// ==== 批次丙 T6：模式切换 ====

/** 统一模式档（与 Rust `inject::mode::MamMode` 的 wire 词一一对应，勿漂移）。
 *  对齐 happy 的 8 值收敛为 MAM 5 值（auto/safe-yolo/yolo 合并为 bypass）。 */
export type MamMode = "plan" | "default" | "acceptEdits" | "bypass" | "readOnly";

/** 模式档中文名（前端渲染；与 Rust `MamMode::label` 同口径） */
export const MAM_MODE_LABELS: Record<MamMode, string> = {
  plan: "计划",
  default: "默认",
  acceptEdits: "接受编辑",
  bypass: "完全信任",
  readOnly: "只读",
};

/** 模式视图（GET /session-mode 载荷）。current=null 表示**档未知**（屏读失败或该
 *  工具不支持回显）→ 前端必须显示「未知」并要求人工核对（红线 4：不假装成功）。
 *  switchKind：unsupported → 不显示切换按钮（该工具无实测机制）。 */
export interface SessionModeView {
  tool: string;
  current: MamMode | null;
  currentLabel: string | null;
  readback: boolean;
  switchKind: "shiftTab" | "slashCommand" | "unsupported";
}

/** 切档回执（POST /session-mode/switch）。verified=false 时 hint 给出人工核对提示
 *  ——切换已投递但无法自动验证（屏读缺失），前端据此渲染提示而非「已切到 X 档」。 */
export type SessionModeSwitchResult =
  | { status: "key_sent"; verified: boolean; hint?: string | null }
  | { status: "failed"; error: string };

/** 拉取当前模式（卡头显示用）。非 2xx → 抛 ApiError（调用方静默降级不显示） */
export async function fetchSessionMode(sessionId: string): Promise<SessionModeView> {
  const q = new URLSearchParams({ session_id: sessionId });
  let r: Response;
  try {
    r = await fetch(`/m/api/v1/session-mode?${q}`);
  } catch (e) {
    throw new ApiError(null, `session-mode 网络异常: ${String(e)}`);
  }
  if (!r.ok) throw new ApiError(r.status, `session-mode ${r.status}`);
  return (await r.json()) as SessionModeView;
}

/** 切档（T6）。404 no_session | 409 no_mechanism → 非 2xx 抛 ApiError */
export async function sessionModeSwitch(
  sessionId: string,
  target: MamMode
): Promise<SessionModeSwitchResult> {
  let r: Response;
  try {
    r = await fetch("/m/api/v1/session-mode/switch", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ sessionId, target }),
    });
  } catch (e) {
    throw new ApiError(null, `session-mode/switch 网络异常: ${String(e)}`);
  }
  if (!r.ok) {
    let data: Record<string, unknown> | null = null;
    try {
      data = (await r.json()) as Record<string, unknown>;
    } catch {
      /* 非 JSON 错误体 */
    }
    throw new ApiError(r.status, `session-mode/switch ${r.status}`, data);
  }
  return (await r.json()) as SessionModeSwitchResult;
}

// ==== M6R–M9R Task 11：一键 resume（R5，在电脑上打开）====

/** 一键 resume 回执（POST /session-open）：200 {status:"opening"} 表示电脑侧正在
 *  打开终端恢复该会话；spawn 出手失败 → 200 {status:"failed",error}（可重试） */
export type SessionOpenResult = { status: "opening" } | { status: "failed"; error: string };

/** 一键 resume（R5）：请求电脑本机打开终端 + cd 项目目录 + 恢复会话 + 聚焦。
 *  404 {error:"no_session"|"no_cwd"|"no_resume_command"} → 非 2xx 抛 ApiError
 *  （错误码解析进 data.error，调用方分診禁用/失败文案——后端命令表未收录的工具
 *  前端按钮本就禁用，404 是挂载后会话漂移的兜底） */
export async function sessionOpen(sessionId: string): Promise<SessionOpenResult> {
  let r: Response;
  try {
    r = await fetch("/m/api/v1/session-open", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ sessionId }),
    });
  } catch (e) {
    throw new ApiError(null, `session-open 网络异常: ${String(e)}`);
  }
  if (!r.ok) {
    // 404 错误码在响应体 data.error——解析进 data 供调用方分診（对齐 sessionApprove 惯例）
    let data: Record<string, unknown> | null = null;
    try {
      data = (await r.json()) as Record<string, unknown>;
    } catch {
      /* 非 JSON 错误体（代理注入页等）：data 保持 null，按 message 兜底 */
    }
    throw new ApiError(r.status, `session-open ${r.status}`, data);
  }
  return (await r.json()) as SessionOpenResult;
}
