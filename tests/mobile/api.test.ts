import { afterEach, describe, expect, it, vi } from "vitest";
import { fetchSessions, pair } from "@/mobile/api";

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
});
