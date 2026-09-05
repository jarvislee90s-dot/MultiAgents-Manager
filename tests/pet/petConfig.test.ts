// tests/pet/petConfig.test.ts
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  loadConfig,
  saveConfig,
  loadVisible,
  saveVisible,
  loadPosition,
  savePosition,
  petSoundTakeover,
  petSuppressPopup,
  subscribeConfig,
} from "@/components/pet/petConfig";
import { saveActiveId, loadActiveName } from "@/components/pet/petRuntime";

describe("petConfig", () => {
  beforeEach(() => localStorage.clear());

  it("无存储时返回默认值（spec §10.1）", () => {
    const c = loadConfig();
    expect(c).toMatchObject({
      alwaysOnTop: true,
      muted: false,
      talkative: true,
      gravity: true,
      scale: 1,
      dblAction: "waving",
      approvalAction: "waiting",
      errorAction: "failed",
      doneAction: "jumping",
    });
    expect(loadVisible()).toBe(false);
    expect(loadPosition()).toBeNull();
  });

  it("非法值回落默认（sanitize）", () => {
    localStorage.setItem(
      "mam-pet-config",
      JSON.stringify({ scale: 9, dblAction: "hack", muted: "yes" })
    );
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

  it("接管判定：开启即接管声音；置顶才抑制浮窗（spec D3/D4）", () => {
    saveVisible(false);
    expect(petSoundTakeover()).toBe(false);
    saveVisible(true);
    expect(petSoundTakeover()).toBe(true);
    saveConfig({ alwaysOnTop: false });
    expect(petSuppressPopup()).toBe(false);
    saveConfig({ alwaysOnTop: true });
    expect(petSuppressPopup()).toBe(true);
  });
});

describe("petConfig 激活指针订阅（P1-6）", () => {
  beforeEach(() => localStorage.clear());

  it("同窗口 saveActiveId 经本地事件即时触发 subscribeConfig 回调", () => {
    const fn = vi.fn();
    const unsub = subscribeConfig(fn);
    fn.mockClear();
    saveActiveId("starry-dew", true, "Starry Dew");
    expect(fn).toHaveBeenCalledTimes(1); // storage 事件只跨窗口：本窗口写入靠本地事件触达
    expect(loadActiveName()).toBe("Starry Dew");
    // 跨窗口 storage 事件同样触达（激活键/展示名键已纳入过滤）
    window.dispatchEvent(new StorageEvent("storage", { key: "mam-pet-active" }));
    expect(fn).toHaveBeenCalledTimes(2);
    window.dispatchEvent(new StorageEvent("storage", { key: "mam-pet-active-name" }));
    expect(fn).toHaveBeenCalledTimes(3);
    unsub();
    saveActiveId("foxbell", true);
    expect(fn).toHaveBeenCalledTimes(3); // 退订后不再触达
  });
});
