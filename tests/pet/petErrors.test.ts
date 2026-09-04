import { describe, expect, it, vi } from "vitest";
import zhLocale from "../../src/i18n/locales/zh.json";
import { isPetRpcError, KNOWN_RPC_CODES, PetError, petErrMsg } from "@/components/pet/petErrors";

// i18n 替身：键 + JSON 参数按 petErrors 约定渲染
const t = vi.fn((k: string, p?: Record<string, unknown>) => {
  if (p && "w" in p) return `${k}:${p.w}x${p.h}`;
  if (p && "name" in p) return `${k}:${p.name}`;
  return k;
});

describe("petErrMsg（P3-6 错误码 → i18n）", () => {
  it("PetError → t(pet.err.<code>, params)", () => {
    const msg = petErrMsg(new PetError("sheet-bad-size", { w: 1536, h: 1000 }), t);
    expect(msg).toBe("pet.err.sheet-bad-size:1536x1000");
    expect(t).toHaveBeenCalledWith("pet.err.sheet-bad-size", { w: 1536, h: 1000 });
  });

  it("PetError 无参数 → t 仅传键", () => {
    expect(petErrMsg(new PetError("sheet-missing"), t)).toBe("pet.err.sheet-missing");
    expect(t).toHaveBeenCalledWith("pet.err.sheet-missing", undefined);
  });

  it("普通 Error → message 透传（含 Rust 侧原样消息）", () => {
    expect(petErrMsg(new Error("宠物不存在: p1"), t)).toBe("宠物不存在: p1");
    expect(t).not.toHaveBeenCalled();
  });

  it("字符串抛出值（Tauri invoke 形态）→ 兜底 scan-fail 键", () => {
    expect(petErrMsg("宠物不存在: p1", t)).toBe("pet.err.scan-fail");
  });

  it("非对象（null/undefined）→ 兜底 scan-fail 键", () => {
    expect(petErrMsg(null, t)).toBe("pet.err.scan-fail");
    expect(petErrMsg(undefined, t)).toBe("pet.err.scan-fail");
  });

  it("message 为空的 Error → 兜底 scan-fail 键", () => {
    expect(petErrMsg(new Error(""), t)).toBe("pet.err.scan-fail");
  });

  describe("PetRpcError 分支（第六轮 Commit 3）", () => {
    it("结构化 RpcError → t(pet.rpc.<code>, params) 正常映射", () => {
      const msg = petErrMsg({ code: "pet-exists", params: { name: "dup" }, detail: "宠物已存在: dup" }, t);
      expect(msg).toBe("pet.rpc.pet-exists:dup");
      expect(t).toHaveBeenCalledWith("pet.rpc.pet-exists", { name: "dup" });
    });

    it("非白名单码 → 显式收敛 pet.rpc.internal 且 detail 进 err（第七轮白名单语义）", () => {
      const t = vi.fn((k: string, p?: Record<string, unknown>) =>
        p && "err" in p ? `${k}:${p.err}` : k
      );
      const msg = petErrMsg({ code: "never-coded", detail: "原始错误原文" }, t);
      expect(msg).toBe("pet.rpc.internal:原始错误原文");
      expect(t).toHaveBeenCalledWith("pet.rpc.internal", { err: "原始错误原文" });
    });

    it("KNOWN_RPC_CODES 与 zh.json 的 pet.rpc.* 键集合一致（防两处漂移）", () => {
      const localeKeys = Object.keys(zhLocale.pet.rpc).sort();
      const codeSet = [...KNOWN_RPC_CODES].sort();
      expect(codeSet).toEqual(localeKeys);
    });

    it("isPetRpcError 类型守卫：仅接受 code 为非空 string 的对象", () => {
      expect(isPetRpcError({ code: "pet-exists" })).toBe(true);
      expect(isPetRpcError({ code: "" })).toBe(false);
      expect(isPetRpcError({ nope: 1 })).toBe(false);
      expect(isPetRpcError("plain string")).toBe(false);
      expect(isPetRpcError(null)).toBe(false);
      expect(isPetRpcError(new Error("x"))).toBe(false); // 普通 Error 无 code 属性
      // PetError 也携带 code 属性：必须显式排除，否则 fatal 分流误走 fatalScan（第八轮）
      expect(isPetRpcError(new PetError("sheet-missing"))).toBe(false);
    });
  });
});
