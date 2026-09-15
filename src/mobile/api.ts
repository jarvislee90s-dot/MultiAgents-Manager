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
