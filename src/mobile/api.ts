// 移动端 API：纯 fetch，零 Tauri 依赖（PWA/浏览器同构）
import type { SessionsResponse, TransitionEvent } from "@/types/session";

export interface PairResult {
  ok: boolean;
}

export async function pair(token: string): Promise<PairResult> {
  const r = await fetch("/m/api/v1/pair", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ token }),
  });
  return { ok: r.ok };
}

export async function fetchSessions<T>(): Promise<T | null> {
  const r = await fetch("/m/api/v1/sessions");
  if (r.status === 403) return null; // 设备失效 → 回配对页
  return r.json() as Promise<T>;
}

// host 载荷（GET /m/api/v1/host）：host 部分对应 Rust host_payload 的 "host" 键；
// enabledTools 为 P8d 受管工具 id 列表（Task 3 chips 过滤的数据源）
export interface HostInfo {
  name: string;
  platform: "macos" | "windows" | "linux";
  version: string;
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
