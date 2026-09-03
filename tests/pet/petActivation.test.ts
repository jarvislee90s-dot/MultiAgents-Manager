import { beforeEach, describe, expect, it, vi } from "vitest";
import { activatePet, buildManifestFromScan, repairManifest } from "@/components/pet/petActivation";
import type { PetScan, PetManifestView } from "@/components/pet/petValidation";
import { tauriInvokeMock } from "../msw/tauriMocks";

// 探测桩：默认 v2 图集、全部音频 3s
vi.mock("@/components/pet/petRuntime", async (importOriginal) => {
  const orig = await importOriginal<typeof import("@/components/pet/petRuntime")>();
  return {
    ...orig,
    probeSheetRows: vi.fn().mockResolvedValue(11),
    probeAudioDurationMs: vi.fn().mockResolvedValue(3000),
  };
});

const scanOf = (files: { rel: string; size: number }[], sheet = 100): PetScan => ({
  id: "p1",
  dir: "/x/p1",
  spritesheet: { rel: "spritesheet.webp", exists: sheet > 0, size: sheet },
  voiceFiles: files.map((f) => ({ rel: f.rel, exists: true, size: f.size })),
});
const g = (n: string) => `voice/${n}/a.mp3`;
const fourGroups = ["general", "approval", "done", "error"].map((n) => ({ rel: g(n), size: 5 }));

describe("activatePet", () => {
  beforeEach(() => {
    localStorage.clear();
    tauriInvokeMock.mockClear();
  });

  it("foxbell：直接写指针并广播", async () => {
    const r = await activatePet("foxbell", async () => "cancel");
    expect(r.status).toBe("activated");
    expect(localStorage.getItem("mam-pet-active")).toBe("foxbell");
    expect(localStorage.getItem("mam-pet-active-name")).toBe("Foxbell");
  });

  it("图集缺失：invalid-sheet，不写指针", async () => {
    tauriInvokeMock.mockImplementation((cmd: string) => {
      if (cmd === "pet_scan") return Promise.resolve(scanOf([], 0));
      if (cmd === "pet_read_manifest") return Promise.resolve(null);
      return Promise.resolve(undefined);
    });
    const r = await activatePet("p1", async () => "cancel");
    expect(r.status).toBe("invalid-sheet");
    expect(localStorage.getItem("mam-pet-active")).toBeNull();
  });

  it("直投无 manifest：全量探测 → 生成 manifest → 激活（spec §6-2）", async () => {
    tauriInvokeMock.mockImplementation((cmd: string) => {
      if (cmd === "pet_scan") return Promise.resolve(scanOf(fourGroups));
      if (cmd === "pet_read_manifest") return Promise.resolve(null);
      return Promise.resolve(undefined);
    });
    const r = await activatePet("p1", async () => "cancel");
    expect(r.status).toBe("activated");
    expect(r.manifestBuilt).toBe(true);
    const call = tauriInvokeMock.mock.calls.find((c) => c[0] === "pet_update_manifest");
    const m = call?.[1]?.manifest as PetManifestView;
    expect(m.hasVoice).toBe(true); // 四组齐 → 有语音
    expect(m.hasSubtitle).toBe(true); // 直投默认有语音即有字幕
    expect(localStorage.getItem("mam-pet-active")).toBe("p1");
    expect(localStorage.getItem("mam-pet-voice-cap")).toBe("1");
  });

  it("音频不全的直投：无语音激活", async () => {
    tauriInvokeMock.mockImplementation((cmd: string) => {
      if (cmd === "pet_scan") return Promise.resolve(scanOf([fourGroups[0]]));
      if (cmd === "pet_read_manifest") return Promise.resolve(null);
      return Promise.resolve(undefined);
    });
    const r = await activatePet("p1", async () => "cancel");
    expect(r.status).toBe("activated");
    expect(localStorage.getItem("mam-pet-voice-cap")).toBe("0");
  });

  it("manifest 不一致 + 用户选更新：备份修复后激活（spec §6-3）", async () => {
    const manifest: PetManifestView = {
      id: "p1", displayName: "P", hasVoice: true, hasSubtitle: true,
      spriteVersionNumber: 2, spritesheetSizeBytes: 100,
      voices: [{ group: "general", name: "a", file: g("general"), sizeBytes: 10, durationMs: 3000 }],
    };
    tauriInvokeMock.mockImplementation((cmd: string) => {
      if (cmd === "pet_scan") return Promise.resolve(scanOf(fourGroups)); // 旧条目 size 10 ≠ 5 → changed；其余 extra
      if (cmd === "pet_read_manifest") return Promise.resolve(manifest);
      return Promise.resolve(undefined);
    });
    const r = await activatePet("p1", async () => "update");
    expect(r.status).toBe("activated");
    expect(r.repaired).toBe(true);
    const call = tauriInvokeMock.mock.calls.find((c) => c[0] === "pet_update_manifest");
    expect(call?.[1]?.backup).toBe(true);
  });

  it("manifest 不一致 + 用户选忽略：无语音降级激活", async () => {
    const manifest: PetManifestView = {
      id: "p1", displayName: "P", hasVoice: false, hasSubtitle: false,
      spriteVersionNumber: 2, spritesheetSizeBytes: 100, voices: [],
    };
    tauriInvokeMock.mockImplementation((cmd: string) => {
      if (cmd === "pet_scan") return Promise.resolve(scanOf([fourGroups[0]], 999)); // 图集也变了
      if (cmd === "pet_read_manifest") return Promise.resolve(manifest);
      return Promise.resolve(undefined);
    });
    const r = await activatePet("p1", async () => "ignore");
    expect(r.status).toBe("activated");
    expect(localStorage.getItem("mam-pet-voice-cap")).toBe("0");
  });

  it("用户选取消：不激活", async () => {
    const manifest: PetManifestView = {
      id: "p1", displayName: "P", hasVoice: false, hasSubtitle: false,
      spriteVersionNumber: 2, spritesheetSizeBytes: 1, voices: [],
    };
    tauriInvokeMock.mockImplementation((cmd: string) => {
      if (cmd === "pet_scan") return Promise.resolve(scanOf([], 999));
      if (cmd === "pet_read_manifest") return Promise.resolve(manifest);
      return Promise.resolve(undefined);
    });
    const r = await activatePet("p1", async () => "cancel");
    expect(r.status).toBe("mismatch");
    expect(localStorage.getItem("mam-pet-active")).toBeNull();
  });
});

describe("buildManifestFromScan / repairManifest", () => {
  it("repair 保留未变条目、重探变动与新增（spec §6-3 修复语义）", async () => {
    const old: PetManifestView = {
      id: "p1", displayName: "Old", hasVoice: true, hasSubtitle: true,
      spriteVersionNumber: 2, spritesheetSizeBytes: 100,
      voices: [
        { group: "general", name: "a", file: g("general"), sizeBytes: 5, durationMs: 3000 }, // 不变
        { group: "approval", name: "b", file: g("approval"), sizeBytes: 99, durationMs: 3000 }, // 变动
      ],
    };
    const repaired = await repairManifest(old, scanOf(fourGroups), 9);
    expect(repaired.spriteVersionNumber).toBe(1); // rows=9 → v1
    expect(repaired.voices.find((v) => v.file === g("general"))?.durationMs).toBe(3000); // 保留缓存
    expect(repaired.voices.find((v) => v.file === g("approval"))?.sizeBytes).toBe(5); // 重探纳入
    expect(repaired.hasVoice).toBe(true);
  });

  it("buildManifest：探测失败的文件排除且不阻断", async () => {
    const { probeAudioDurationMs } = await import("@/components/pet/petRuntime");
    vi.mocked(probeAudioDurationMs).mockRejectedValueOnce(new Error("x"));
    const m = await buildManifestFromScan("p1", scanOf(fourGroups), 11, "petdex", false);
    expect(m.voices).toHaveLength(3); // 一个探测失败被排除
    expect(m.hasVoice).toBe(false); // 该组无覆盖
    expect(m.spriteVersionNumber).toBe(2);
    expect(m.source).toBe("petdex");
  });
});