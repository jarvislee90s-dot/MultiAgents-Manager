import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError, fetchHost, fetchSessions, pairWithPin } from "@/mobile/api";

// setup.ts 的 msw server 会包一层全局 fetch；stub 覆盖其上，结束后还原防止泄漏到其他用例
afterEach(() => {
  vi.unstubAllGlobals();
});

describe("mobile api", () => {
  it("pairWithPin POST /pair/pin 携带 pin，成功透传 ok", async () => {
    const f = vi.fn(async () => new Response('{"ok":true}', { status: 200 }));
    vi.stubGlobal("fetch", f);
    expect((await pairWithPin("4827")).ok).toBe(true);
    expect(f).toHaveBeenCalledWith(
      "/m/api/v1/pair/pin",
      expect.objectContaining({
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ pin: "4827" }),
      })
    );
  });

  it("pairWithPin 错误体 JSON 挂到 ApiError.data（error/remaining/retryAfter 分診依据）", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response('{"error":"invalid_pin","remaining":2}', { status: 401 }))
    );
    const err = await pairWithPin("0000").catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).status).toBe(401);
    expect((err as ApiError).data).toEqual({ error: "invalid_pin", remaining: 2 });
  });

  it("pairWithPin 网络异常 → ApiError status=null（PairPage 网络文案分診依据）", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new TypeError("network down");
      })
    );
    const err = await pairWithPin("4827").catch((e: unknown) => e);
    expect((err as ApiError).status).toBeNull();
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
});
