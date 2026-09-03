import { beforeEach, describe, expect, it } from "vitest";
import { loadActiveId, saveActiveId, loadVoiceCap, rowsFromSize, FOXBELL } from "@/components/pet/petRuntime";

describe("petRuntime", () => {
  beforeEach(() => localStorage.clear());

  it("激活指针：默认 foxbell，可读写", () => {
    expect(loadActiveId()).toBe("foxbell");
    saveActiveId("starry-dew", false, "Starry Dew");
    expect(loadActiveId()).toBe("starry-dew");
    expect(loadVoiceCap()).toBe(false);
  });

  it("rowsFromSize：v1/v2 识别与非法尺寸（EP1）", () => {
    expect(rowsFromSize(1536, 1872)).toBe(9);
    expect(rowsFromSize(1536, 2288)).toBe(11);
    expect(rowsFromSize(1536, 1000)).toBeNull();
    expect(rowsFromSize(1024, 1872)).toBeNull();
  });

  it("FOXBELL 描述符：内置路径与全能力", () => {
    expect(FOXBELL.id).toBe("foxbell");
    expect(FOXBELL.rows).toBe(11);
    expect(FOXBELL.hasVoice).toBe(true);
    expect(FOXBELL.resolveVoiceUrl("a.m4a")).toBe("/pet/voice/a.m4a");
  });
});