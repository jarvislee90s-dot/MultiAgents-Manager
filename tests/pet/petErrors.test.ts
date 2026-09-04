import { describe, expect, it, vi } from "vitest";
import { PetError, petErrMsg } from "@/components/pet/petErrors";

// i18n 替身：键 + JSON 参数按 petErrors 约定渲染
const t = vi.fn((k: string, p?: Record<string, unknown>) => {
  if (p && "w" in p) return `${k}:${p.w}x${p.h}`;
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
});