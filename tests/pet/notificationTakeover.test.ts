// tests/pet/notificationTakeover.test.ts — 通知让渡判定（spec D3/D4）
import { describe, expect, it, beforeEach } from "vitest";
import { petSoundTakeover, petSuppressPopup } from "@/components/pet/petConfig";

describe("通知让渡判定（spec D3/D4）", () => {
  beforeEach(() => localStorage.clear());
  it("宠物关闭：不接管、不抑制", () => {
    localStorage.setItem("mam-pet-visible", "0");
    expect(petSoundTakeover()).toBe(false);
    expect(petSuppressPopup()).toBe(false);
  });
  it("宠物开启即接管声音；置顶才抑制浮窗", () => {
    localStorage.setItem("mam-pet-visible", "1");
    expect(petSoundTakeover()).toBe(true);
    localStorage.setItem("mam-pet-config", JSON.stringify({ alwaysOnTop: false }));
    expect(petSuppressPopup()).toBe(false);
    localStorage.setItem("mam-pet-config", JSON.stringify({ alwaysOnTop: true }));
    expect(petSuppressPopup()).toBe(true);
  });

  describe("语音能力闸门（spec §5.2）", () => {
    it("无语音外部宠物不接管完成提示音", () => {
      localStorage.setItem("mam-pet-visible", "1");
      localStorage.setItem("mam-pet-voice-cap", "0");
      expect(petSoundTakeover()).toBe(false);
    });
    it("foxbell（未写能力缓存）保持接管", () => {
      localStorage.setItem("mam-pet-visible", "1");
      localStorage.removeItem("mam-pet-voice-cap");
      expect(petSoundTakeover()).toBe(true);
    });
  });
});
