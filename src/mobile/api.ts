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
 *  kind ∈ user / assistant / thinking / tool-call / tool-result；
 *  thinking 与 tool-call 的 collapsed 恒 true（wire 语义，运行中态的默认折叠依据） */
export interface SessionMessage {
  seq: number;
  role: string;
  content: string;
  kind: "user" | "assistant" | "thinking" | "tool-call" | "tool-result" | string;
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
 *  不自动轮询（M3 范围裁决：SSE transition 不驱动详情页，下拉手动刷新） */
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
 *  前端据此把「仅读过」的文件名渲成常规色，与「动过手」的区分） */
export interface SessionFileEntry {
  path: string;
  lastSeq: number;
  lastTs: number | null;
  hits: number;
  modified: boolean;
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
  if (!r.ok) throw new ApiError(r.status, `file ${r.status}`);
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
