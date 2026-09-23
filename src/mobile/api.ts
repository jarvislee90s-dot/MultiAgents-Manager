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
  /** 丁T2：**计划待确认预期态**（消息尾部派生，无新存储）——codex/kimi 的计划确认框
   *  不落状态/标记，这是它唯一的可见信号。true 且 `dialog=false` 时前端渲染
   *  「计划待确认」条 +「检查终端对话框」按钮（点它重拉本端点；后端屏读命中即出
   *  N 选项）；此形态下 options 恒空**不是错误**，是「还没读到选项，点检查重试」。
   *  缺省/false → 既有渲染（前向兼容旧后端） */
  planPending?: boolean;
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
  /** 丁T5 §2.4：卡内自由作答输入框是否可用（**独立于 `answerable`**——
   *  codex/opencode 的**点选**已实测可作答，但**自由作答序列未定案**）。
   *  缺省/旧后端 → 按 false 处理：渲染「请在终端作答」引导文案，**不假装能发**。 */
  freeText?: boolean;
  /** 批次戊 E4-E6：多题交互能力（kimi/codex/opencode 已实机定案）——true 时多题卡
   *  渲染逐题作答 UI；缺省/旧后端/false → 只读卡（红线不变） */
  multiQuestion?: boolean;
  /** 切换题目能力（2026-09-23 错位修复）：多题卡多选题的显式切页动作——仅 opencode
   *  （tab=前向切页，实测定案）。缺省/旧后端 → 按 false：多选题不渲染「切换题目」钮，
   *  改渲染「请到终端切题」引导（不假装能发）。 */
  advance?: boolean;
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
 *  submit=多选提交（**阶段机闭环**：屏读确认每段后才推进）；
 *  cancel=取消问题（Esc）；freeText=自由作答（**仅 claude**，阶段机闭环：
 *  定位 `Type something` 行 → 文本 → 回车）；
 *  advance=多题切换题目（**仅 opencode**，tab 前向切页——纯导航，不触碰勾选态）。 */
export type QuestionAnswerAction =
  "select" | "toggle" | "submit" | "cancel" | "freeText" | "advance";

/** 阶段机动作的**段名**（回执 `stage` 字段的取值；与后端
 *  `remote::api::QUESTION_STAGE_*` 常量逐字对应，勿漂移）。
 *  提交链推进序：`submit-row`→`review`→`confirm`→`receipt`；
 *  自由作答：`free-row`→`free-text`。 */
export type QuestionAnswerStage =
  "submit-row" | "review" | "confirm" | "receipt" | "free-row" | "free-text";

/** 问答应答回执（POST /session-question/answer 响应，HTTP 200 恒定，语义在 body.status）。
 *
 *  **丁T5 起 status 仍是既有两词**（`key_sent` / `failed`），新增字段全部是**附加**
 *  ——故旧前端（只读 status）行为不变：
 *  - `key_sent`：按键已投递。**单键动作**（select/toggle/cancel）到此为止；
 *    **阶段机动作**（submit/freeText）走完整条闭环时带 `done:true` + `stage`（走完的
 *    段）+ `verified`（终态回执三态：true=屏读到终态锚；false=读到屏但未见锚；
 *    null/缺省=读屏不可用。**false 与 null 都不是「失败」，是「未确认」**）；
 *  - `failed`：投递失败 / in-flight 忙让位 / **阶段机中止**。`aborted:true` + `stage`
 *    标记后者（`error` 是带段名的中文文案，用户可读）。 */
export type QuestionAnswerResult =
  | { status: "key_sent"; done?: boolean; stage?: QuestionAnswerStage; verified?: boolean | null }
  | { status: "failed"; error: string; aborted?: boolean; stage?: QuestionAnswerStage };

/** 问答应答**错误码 → 用户可读中文文案**（丁T6 复评抽出：卡内与 composer 两条入口
 *  必须**同口径**——两处各写一套 `if/else` 迟早漂移，且 composer 侧曾漏掉这条映射
 *  （读的是 `data.reason` 而问答端点回的是 `data.error`）→ 409 会显示成
 *  「session-question/answer 409」这种对用户无意义的串）。
 *
 *  取值来源：后端 `remote::api::session_question_answer` 的 409/400 错误码
 *  （`no_question` / `multi_questions` / `tool_readonly` / `bad_index`；
 *  另有 `bad_request` 兜底）。码不在表内 → 回原 message（不编文案）。 */
export function questionAnswerErrorCopy(e: ApiError): string {
  const code = typeof e.data?.error === "string" ? e.data.error : null;
  if (code === "no_question") return "当前没有待回答的问题";
  if (code === "multi_questions") return "多个问题请回到终端完成作答";
  if (code === "tool_readonly") return "该工具的远程作答尚未实测，请在终端完成作答";
  if (code === "bad_index") return "选项序号无效，请刷新后重试";
  return e.message;
}

/** 问答一键应答（T8；丁T5 起支持 freeText）。index = 选项序号（0 起；select/toggle
 *  必填）；text = 自由作答正文（freeText 必填；后端归一后走**字符通道**注入，
 *  不带 `[mobile]` 签名）。
 *  409 {error:"no_question"|"multi_questions"|"tool_readonly"} |
 *  400 {error:"bad_request"|"bad_index"}
 *  → 非 2xx 抛 ApiError（错误码解析进 data.error，调用方分診中文文案——
 *  用 [`questionAnswerErrorCopy`]，勿另写一套） */
export async function sessionQuestionAnswer(
  sessionId: string,
  action: QuestionAnswerAction,
  index?: number,
  text?: string,
  /** 批次戊 E4-E6 多题交互：select/toggle 作用在第几题（0 起） */
  questionIndex?: number
): Promise<QuestionAnswerResult> {
  let r: Response;
  try {
    r = await fetch("/m/api/v1/session-question/answer", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ sessionId, action, index, text, questionIndex }),
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

// 注（T4 复评 M4）：批次丙 T6 的 `MAM_MODE_LABELS` 通用档名表已删除——丁T4 起
// 按钮标签一律用**后端下发的屏显标签**（`groups[].tiers[].label`，§2.6：标签必须
// 是工具自己的词，如 kimi 权限组的「总是询问/按需询问/永不询问」），通用档名只剩
// 回执文案里的兜底（`MamMode::label`，后端侧）。前端再留一份 = 第二份真源 + 死代码。

/** 模式栏的**组**（丁T4 §2.6：二维工具的「模式组/权限组」与单轴工具的「模式」轴） */
export type ModeGroupId = "mode" | "permission";

/** 单档（GET 载荷 `groups[].tiers[]`）：屏显标签来自**工具自己的词表**（§2.6
 *  「档位（屏显标签）」列）——kimi 权限组是「总是询问/按需询问/永不询问」，不是 MAM
 *  通用名（用户看到的是终端上的词，对不上号等于没回显）。
 *  `selectable=false` → 不渲染为可点按钮（`reason` 是后端给出的如实原因）。 */
export interface ModeTierView {
  mode: MamMode;
  label: string;
  selectable: boolean;
  reason?: string | null;
}

/** 已退役旧档（裁7）：**不可选**，只作如实展示（codex 的 untrusted / on-failure） */
export interface ModeLegacyView {
  label: string;
  note: string;
}

/** 单组（GET 载荷 `groups[]`）。`step=true` = 步进轴（shift+tab 一次一档，档位顺序即
 *  实测环序）；`readback=false` = 该组无屏读源（前端必须显示「请人工核对」）。
 *  `current=null` = 档未知（屏读失败或该组无回读源）→ **不得假装知道**（红线 4）。
 *  `layout`（2026-09-23 codex 模式切换改造）：`"toggle"` = 单钮循环（点击向终端发一次
 *  循环键——codex 模式组「计划 ⇄ 操作」= shift+tab，目标档由前端按当前档翻转）；
 *  缺省/`"tiers"` = 逐档按钮；`"picker"` = **单选面板**（codex 权限组，2026-09-23 用户
 *  方案）：单钮「切换权限」→ 后端读回**终端菜单的选项表**（编号 = 屏上实读值）→ 用户
 *  点选哪项，MAM 就敲哪个数字键——前端**不再硬编码「哪档对应哪个数字」**。 */
export interface ModeGroupView {
  id: ModeGroupId;
  label: string;
  step: boolean;
  readback: boolean;
  layout?: "tiers" | "toggle" | "picker";
  current: MamMode | null;
  currentLabel: string | null;
  tiers: ModeTierView[];
  legacy?: ModeLegacyView[];
}

/** 模式视图（GET /session-mode 载荷）。current=null 表示**档未知**（屏读失败或该
 *  工具不支持回显）→ 前端必须显示「未知」并要求人工核对（红线 4：不假装成功）。
 *  switchKind：unsupported → 不显示切换按钮（该工具无实测机制）。
 *
 *  丁T4 增量（**全部可选**，与旧后端前向兼容）：`structure`/`groups` 缺失时前端回落
 *  到「单轴渲染 + 顶层 current」。 */
export interface SessionModeView {
  tool: string;
  current: MamMode | null;
  currentLabel: string | null;
  readback: boolean;
  switchKind: "shiftTab" | "slashCommand" | "unsupported";
  /** E3④：终端问答待决（消息尾部形态）——切档注入含回车会被问答框误消费
   *  （codex 交默认答案 / kimi 误选推进待决态）→ 前端置灰按钮 + 原因文案。
   *  旧后端无此字段（undefined = 未知，不置灰——与「无法判定放行」同一取向）。 */
  questionPending?: boolean;
  /** "twoAxis" | "singleAxis" | "none"（旧后端无此字段） */
  structure?: "twoAxis" | "singleAxis" | "none";
  groups?: ModeGroupView[];
}

/** 切档回执（POST /session-mode/switch）。verified=false 时 hint 给出人工核对提示
 *  ——切换已投递但无法自动验证（屏读缺失），前端据此渲染提示而非「已切到 X 档」。
 *  丁T3：`dialogChecked` = 本次是否真的做过对话框在场检测（false = 平台无屏读或
 *  屏读失败；此时守卫按「无法判定」放行，前端不得声称已检查）。
 *  丁T4：`current`/`currentLabel` = 投递后回读到的档（命中时前端可直接用它刷新）。 */
export type SessionModeSwitchResult =
  | {
      status: "key_sent";
      verified: boolean;
      hint?: string | null;
      dialogChecked?: boolean;
      current?: MamMode | null;
      currentLabel?: string | null;
    }
  | { status: "failed"; error: string; dialogChecked?: boolean };

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

/** 切档（T6；丁T4 加 `group`）。404 no_session | 409 no_mechanism |
 *  **409 blocked_by_dialog**（丁T3 §2.7 对话框在场红线：控制类注入被拒，data.reason
 *  为中文文案）→ 非 2xx 抛 ApiError。
 *
 *  `group` 是**可选**参数（丁T4）：不传 = 由后端按 target 归组（旧客户端的调用面，
 *  语义见 Rust `inject::mode::resolve_group`）；二维工具（codex/kimi）的新前端传它。 */
export async function sessionModeSwitch(
  sessionId: string,
  target: MamMode,
  group?: ModeGroupId
): Promise<SessionModeSwitchResult> {
  let r: Response;
  try {
    r = await fetch("/m/api/v1/session-mode/switch", {
      method: "POST",
      headers: { "content-type": "application/json" },
      // group 缺省时**不发字段**（旧后端不认识它，发了也只是被 serde 忽略——
      // 但省掉字段可让请求体与旧版本逐字一致，便于抓包比对）
      body: JSON.stringify(
        group === undefined ? { sessionId, target } : { sessionId, target, group }
      ),
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

// ==== 2026-09-23：codex 权限组的「终端菜单单选题」（用户方案）====

/** 终端菜单里的一项。`number` = **屏上实读的编号**（用户点它 → MAM 敲同一个数字键）；
 *  `label` = 屏上原文（原样展示，供用户与终端核对）；`highlighted` = 终端当前高亮项。 */
export interface ModeMenuOption {
  number: number;
  label: string;
  highlighted: boolean;
}

/** 终端菜单面板的载荷（POST /session-mode/menu 的 `open`/`pick`，
 *  与 GET /session-mode/menu 的重读同形）。
 *
 *  - `menu`：菜单开着，`options` = 档位表（用户点选）；
 *  - `confirm`：Full Access 的**二阶段风险确认框**在屏，`options` = 确认框选项
 *    （空数组 + `hint` = 确认框在屏但选项未读到，提示重读）；
 *  - `done`：已投递且无确认框。`verified` = 屏上是否读到成功回执行；
 *    `hint` 原样带出后端文案（含回执行原文）——**不假装成功**（目标档未知：
 *    用户点的是屏上编号，后端不知道对应哪个 wire 档，故只报「有没有回执」）；
 *  - `none`：屏上无菜单/确认框（仅 GET 重读会给）；
 *  - `failed`：如实失败文案。 */
export type ModeMenuResult =
  | { status: "menu"; options: ModeMenuOption[]; dialogChecked?: boolean }
  | {
      status: "confirm";
      options: ModeMenuOption[];
      hint?: string | null;
      dialogChecked?: boolean;
    }
  | {
      status: "done";
      verified: boolean;
      hint?: string | null;
      dialogChecked?: boolean;
    }
  | { status: "none"; options: ModeMenuOption[] }
  | { status: "failed"; error: string };

/** 面板错误体（非 2xx）→ 解析进 ApiError.data（错误码分诊：no_session / no_mechanism
 *  / blocked_by_dialog），与 `sessionModeSwitch` 同口径。 */
async function modeMenuFetch(init: RequestInit, path: string): Promise<ModeMenuResult> {
  let r: Response;
  try {
    r = await fetch(path, init);
  } catch (e) {
    throw new ApiError(null, `session-mode/menu 网络异常: ${String(e)}`);
  }
  if (!r.ok) {
    let data: Record<string, unknown> | null = null;
    try {
      data = (await r.json()) as Record<string, unknown>;
    } catch {
      /* 非 JSON 错误体 */
    }
    throw new ApiError(r.status, `session-mode/menu ${r.status}`, data);
  }
  return (await r.json()) as ModeMenuResult;
}

/** **打开终端权限菜单**并读回选项表（注入 `/permissions` + 回车）。
 *  非 2xx → ApiError（409 blocked_by_dialog 等，与切档端点同分诊）。 */
export async function sessionModeMenuOpen(sessionId: string): Promise<ModeMenuResult> {
  return modeMenuFetch(
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ sessionId, action: "open" }),
    },
    "/m/api/v1/session-mode/menu"
  );
}

/** **按用户点选的屏上编号敲键**（无回车）。硬前置由后端把关：屏上没有菜单/确认框 →
 *  零投递并如实报错（`failed` 或抛 ApiError）。 */
export async function sessionModeMenuPick(
  sessionId: string,
  number: number
): Promise<ModeMenuResult> {
  return modeMenuFetch(
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ sessionId, action: "pick", number }),
    },
    "/m/api/v1/session-mode/menu"
  );
}

/** **重新读取**（纯屏读、零注入）：把面板与终端当前屏面对齐。用于用户手动在终端开了
 *  菜单、或上一步读屏竞态时。 */
export async function fetchSessionModeMenu(sessionId: string): Promise<ModeMenuResult> {
  const q = new URLSearchParams({ session_id: sessionId });
  return modeMenuFetch({ method: "GET" }, `/m/api/v1/session-mode/menu?${q}`);
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

// ==== 历史会话区（spec 2026-09-20-mobile-archive-history §6.1）====
export interface ArchivedSession {
  sessionId: string;
  agentType: string;
  projectPath: string;
  projectName: string;
  title: string | null;
  lastStatus: string;
  lastSeenAt: string;
  /** 软归档活会话标记（体验批二）：true = 看板隐藏中的活会话（APP 形态），
   *  详情页动作是「移回看板」而非「在桌面端打开」 */
  hiddenAlive?: boolean;
}

export interface ArchivedPayload {
  archived: ArchivedSession[];
  projects: string[];
}

/** 懒加载归档列表（进入历史页/切换天数时调用；403 → null 回配对页） */
export async function fetchArchivedSessions(days: 1 | 3 | 7): Promise<ArchivedPayload | null> {
  const r = await fetch(`/m/api/v1/sessions-archived?days=${days}`);
  if (r.status === 403) return null;
  if (!r.ok) {
    throw new ApiError(r.status, `sessions-archived ${r.status}`);
  }
  return r.json() as Promise<ArchivedPayload>;
}

/** 归档手动管理（spec 裁决 8）：带 id = 单条移除；缺省 = 清空全部 */
export async function deleteArchivedSession(sessionId?: string): Promise<number> {
  const qs = sessionId ? `?session_id=${encodeURIComponent(sessionId)}` : "?all=1";
  const r = await fetch(`/m/api/v1/sessions-archived${qs}`, { method: "DELETE" });
  if (!r.ok) throw new ApiError(r.status, `sessions-archived DELETE ${r.status}`);
  const data = (await r.json()) as { deleted: number };
  return data.deleted;
}

// ==== 看板关闭/软归档（2026-09-20 体验批二）====

/** {sessionId} 请求体（close/hide/unhide 三端点共用） */
function sessionActionBody(sessionId: string): string {
  return JSON.stringify({ sessionId });
}

/** 远程硬杀 CLI 会话终端（桌面 kill_session 同内核）：进程死 → 3s 内下板 → 进历史归档。
 *  App 形态端点拒绝（400 form_not_supported）——软归档走 hideSession */
export async function closeSession(sessionId: string): Promise<void> {
  const r = await fetch("/m/api/v1/session-close", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: sessionActionBody(sessionId),
  });
  if (!r.ok) throw new ApiError(r.status, `session-close ${r.status}`);
}

/** APP 形态软归档（看板隐藏，不杀进程、可逆）：任意状态可归档（叉不挑颜色），
 *  等同桌面端叉掉——不自动回归，恢复唯一路径 = 历史页「移回看板」（unhideSession）；
 *  CLI 会话端点拒绝（400 form_not_supported）——硬杀走 closeSession */
export async function hideSession(sessionId: string): Promise<void> {
  const r = await fetch("/m/api/v1/session-hide", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: sessionActionBody(sessionId),
  });
  if (!r.ok) throw new ApiError(r.status, `session-hide ${r.status}`);
}

/** 解除软归档（移回看板）。幂等：不在隐藏集也 ok（removed=0） */
export async function unhideSession(sessionId: string): Promise<void> {
  const r = await fetch("/m/api/v1/session-unhide", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: sessionActionBody(sessionId),
  });
  if (!r.ok) throw new ApiError(r.status, `session-unhide ${r.status}`);
}
