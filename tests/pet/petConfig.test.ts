// tests/pet/petConfig.test.ts
import { beforeEach, describe, expect, it } from "vitest";
import {
  loadConfig, saveConfig, loadVisible, saveVisible,
  loadPosition, savePosition, petSoundTakeover, petSuppressPopup,
} from "@/components/pet/petConfig";

describe("petConfig", () => {
  beforeEach(() => localStorage.clear());

  it("无存储时返回默认值（spec §10.1）", () => {
    const c = loadConfig();
    expect(c).toMatchObject({
      alwaysOnTop: true, muted: false, talkative: true, gravity: true, scale: 1,
      dblAction: "waving", approvalAction: "waiting", errorAction: "failed", doneAction: "jumping",
    });
    expect(loadVisible()).toBe(false);
    expect(loadPosition()).toBeNull();
  });

  it("非法值回落默认（sanitize）", () => {
    localStorage.setItem("mam-pet-config", JSON.stringify({ scale: 9, dblAction: "hack", muted: "yes" }));
    const c = loadConfig();
    expect(c.scale).toBe(1);
    expect(c.dblAction).toBe("waving");
    expect(c.muted).toBe(false);
  });

  it("patch 保存与读取回环", () => {
    saveConfig({ scale: 1.25, doneAction: "review" });
    expect(loadConfig().scale).toBe(1.25);
    expect(loadConfig().doneAction).toBe("review");
  });

  it("visible / position 回环", () => {
    saveVisible(true);
    expect(loadVisible()).toBe(true);
    savePosition({ x: 100.6, y: -200 });
    expect(loadPosition()).toEqual({ x: 101, y: -200 });
  });

  // 旧宠物 spec D3/D4 以「置顶」为浮窗抑制条件；特性 spec W1（2026-09-03 §2）改为仅看可见性，以 W1 为准
  it("接管判定：可见即接管声音并抑制浮窗（无论置顶，spec W1）", () => {
    saveVisible(false);
    expect(petSoundTakeover()).toBe(false);
    expect(petSuppressPopup()).toBe(false);
    saveVisible(true);
    expect(petSoundTakeover()).toBe(true);
    saveConfig({ alwaysOnTop: false });
    expect(petSuppressPopup()).toBe(true);
    saveConfig({ alwaysOnTop: true });
    expect(petSuppressPopup()).toBe(true);
  });
});
