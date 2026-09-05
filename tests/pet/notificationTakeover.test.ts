// tests/pet/notificationTakeover.test.ts — 通知让渡判定
// 旧宠物 spec（2026-09-01）D3/D4 以「置顶」为浮窗抑制条件；当前特性 spec W1
// （2026-09-03-workbuddy-app-jump-tool-toggle-design.md §2）已改为「宠物可见即抑制浮窗 + 接管声音」，
// 以 W1 为准（气泡是唯一通知面，抑制条件仅看可见性）
import { describe, expect, it, beforeEach } from "vitest";
import { petSoundTakeover, petSuppressPopup } from "@/components/pet/petConfig";

describe("通知让渡判定（spec W1）", () => {
  beforeEach(() => localStorage.clear());
  it("宠物关闭：不接管、不抑制", () => {
    localStorage.setItem("mam-pet-visible", "0");
    expect(petSoundTakeover()).toBe(false);
    expect(petSuppressPopup()).toBe(false);
  });
  it("宠物可见即接管声音并抑制浮窗（无论置顶，spec W1）", () => {
    localStorage.setItem("mam-pet-visible", "1");
    expect(petSoundTakeover()).toBe(true);
    // 非置顶也抑制：W1 去掉了旧 D4 的 alwaysOnTop 条件
    localStorage.setItem("mam-pet-config", JSON.stringify({ alwaysOnTop: false }));
    expect(petSuppressPopup()).toBe(true);
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
