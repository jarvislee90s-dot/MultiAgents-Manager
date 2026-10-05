import { describe, expect, it, vi } from "vitest";
import zhLocale from "../../src/i18n/locales/zh.json";
import enLocale from "../../src/i18n/locales/en.json";
import { isUsageRpcError, KNOWN_USAGE_CODES, usageErrMsg } from "@/components/usage/usageErrors";

// i18n 替身：键 + JSON 参数按 usageErrors 约定渲染（detail 进 {{err}}）
const t = vi.fn((k: string, p?: Record<string, unknown>) =>
  p && "err" in p ? `${k}:${p.err}` : k
);

describe("usageErrMsg（用量域错误码 → i18n）", () => {
  it("KNOWN_USAGE_CODES 与 zh/en 的 usage.rpc.* 键集合一致（码表各处同步、不漂移）", () => {
    const codeSet = [...KNOWN_USAGE_CODES].sort();
    expect(codeSet).toEqual(Object.keys(zhLocale.usage.rpc).sort());
    expect(codeSet).toEqual(Object.keys(enLocale.usage.rpc).sort());
  });

  it("除状态型两码外，全部模板都带 {{err}}（否则 detail 会被静默吞掉 —— W-28）", () => {
    // 例外是**刻意**的：这两个码描述状态（维度非法 / 总开关关闭），文案本身不需要 err 插值。
    // 例外集写死在断言里 → 新增码若漏了 {{err}} 会红；把例外码补上 {{err}} 也会红（迫使显式改这一行）。
    const detailFree = ["usage-disabled", "usage-groupby-invalid"];
    for (const [name, locale] of [
      ["zh", zhLocale.usage.rpc],
      ["en", enLocale.usage.rpc],
    ] as const) {
      for (const code of KNOWN_USAGE_CODES) {
        const tpl = (locale as Record<string, string>)[code];
        expect(typeof tpl, `${name}.${code} 必须存在`).toBe("string");
        if (detailFree.includes(code)) {
          expect(tpl, `${name}.${code} 是状态型文案，不该有 {{err}}`).not.toContain("{{err}}");
        } else {
          expect(tpl, `${name}.${code} 必须含 {{err}}（W-28：detail 要能定位到源/文件）`).toContain(
            "{{err}}"
          );
        }
      }
    }
  });

  it("结构化错误 → t(usage.rpc.<code>, {err: detail})：detail 原样透出（W-28）", () => {
    const e = {
      code: "usage-source-io",
      detail: "/Users/x/.claude/projects/-p-a/sess.jsonl: Permission denied (os error 13)",
    };
    expect(usageErrMsg(e, t)).toBe(`usage.rpc.usage-source-io:${e.detail}`);
    expect(t).toHaveBeenCalledWith("usage.rpc.usage-source-io", { err: e.detail });
  });

  it("未知码 → 显式收敛 usage-internal（不依赖 i18next 缺键回键名），detail 仍透出", () => {
    expect(usageErrMsg({ code: "usage-never-coded", detail: "原始错误原文" }, t)).toBe(
      "usage.rpc.usage-internal:原始错误原文"
    );
    expect(usageErrMsg({ code: "usage-never-coded" }, t)).toBe("usage.rpc.usage-internal:");
  });

  it("缺 detail 的已知码 → err 为空串（模板仍可渲染，不出现 undefined）", () => {
    expect(usageErrMsg({ code: "usage-source-db-open" }, t)).toBe(
      "usage.rpc.usage-source-db-open:"
    );
  });

  it("非结构化错误：Error → message 透传；字符串/空对象 → 兜底 internal", () => {
    expect(usageErrMsg(new Error("DB 锁中毒"), t)).toBe("DB 锁中毒");
    expect(usageErrMsg("宠物不存在: p1", t)).toBe("usage.rpc.usage-internal:");
    expect(usageErrMsg(null, t)).toBe("usage.rpc.usage-internal:");
    expect(usageErrMsg(undefined, t)).toBe("usage.rpc.usage-internal:");
    expect(usageErrMsg({ nope: 1 }, t)).toBe("usage.rpc.usage-internal:");
  });

  it("isUsageRpcError：只接受 code 为非空 string 的对象", () => {
    expect(isUsageRpcError({ code: "usage-source-io" })).toBe(true);
    expect(isUsageRpcError({ code: "" })).toBe(false);
    expect(isUsageRpcError({ code: 7 })).toBe(false);
    expect(isUsageRpcError(new Error("x"))).toBe(false);
    expect(isUsageRpcError("usage-source-io")).toBe(false);
    expect(isUsageRpcError(null)).toBe(false);
  });
});
