import { afterEach, describe, expect, it, vi } from "vitest";
import {
  confirmPairing,
  fetchHost,
  fetchSessions,
  pair,
  pollPairing,
  requestPairing,
} from "@/mobile/api";

// setup.ts 的 msw server 会包一层全局 fetch；stub 覆盖其上，结束后还原防止泄漏到其他用例
afterEach(() => {
  vi.unstubAllGlobals();
});

describe("mobile api", () => {
  it("pair 提交 token 并透传成败", async () => {
    const f = vi.fn(async () => new Response("{}", { status: 200 }));
    vi.stubGlobal("fetch", f);
    expect((await pair("tok")).ok).toBe(true);
    expect(f).toHaveBeenCalledWith("/m/api/v1/pair", expect.objectContaining({ method: "POST" }));
  });

  it("403 返回 null 触发回配对页", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("", { status: 403 }))
    );
    expect(await fetchSessions()).toBeNull();
  });

  it("fetchHost GET /m/api/v1/host，透传载荷；403 同样返回 null（设备失效语义一致）", async () => {
    const host = {
      host: { name: "JARVIS-Win", platform: "windows", version: "0.4.1" },
      enabledTools: ["claude"],
    };
    const f = vi.fn(async () => new Response(JSON.stringify(host), { status: 200 }));
    vi.stubGlobal("fetch", f);
    expect(await fetchHost<typeof host>()).toEqual(host);
    expect(f).toHaveBeenCalledWith("/m/api/v1/host");

    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("", { status: 403 }))
    );
    expect(await fetchHost()).toBeNull();
  });

  // M4 T2：请求接入三端点封装
  it("requestPairing parses 200 and maps 429", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValueOnce(
      new Response('{"requestId":"r0","expiresAt":1}', { status: 200 })
    );
    const r = await requestPairing("我的手机");
    expect(r.requestId).toBe("r0");
    fetchMock.mockResolvedValueOnce(new Response('{"error":"queue_full"}', { status: 429 }));
    await expect(requestPairing("x")).rejects.toMatchObject({ status: 429, message: "queue_full" });
  });

  it("pollPairing returns status strings", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValueOnce(
      new Response('{"status":"pending","expiresAt":1}', { status: 200 })
    );
    expect(await pollPairing("r0")).toBe("pending");
    fetchMock.mockResolvedValueOnce(new Response('{"status":"approved"}', { status: 200 }));
    expect(await pollPairing("r0")).toBe("approved");
  });

  it("confirmPairing surfaces wrong + triesLeft", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    fetchMock.mockResolvedValueOnce(
      new Response('{"ok":false,"error":"wrong","triesLeft":2}', { status: 200 })
    );
    expect(await confirmPairing("r0", "0000")).toEqual({ ok: false, error: "wrong", triesLeft: 2 });
  });
});
