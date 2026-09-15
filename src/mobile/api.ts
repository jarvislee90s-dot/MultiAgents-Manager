// 移动端 API：纯 fetch，零 Tauri 依赖（PWA/浏览器同构）
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
