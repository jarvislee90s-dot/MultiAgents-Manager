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

  it("manifest 不一致 + 用户选忽略：voice-cap 按 manifest 条目在磁盘的存在性判定（FIX-3）", async () => {
    const manifest: PetManifestView = {
      id: "p1", displayName: "P", hasVoice: true, hasSubtitle: true,
      spriteVersionNumber: 2, spritesheetSizeBytes: 100,
      voices: [
        { group: "general", name: "a", file: g("general"), sizeBytes: 10, durationMs: 3000 },
        { group: "done", name: "b", file: g("done"), sizeBytes: 10, durationMs: 3000 },
      ],
    };
    tauriInvokeMock.mockImplementation((cmd: string) => {
      // 磁盘仅 general 一条（done 缺失）→ 任一缺失 → voice-cap false
      if (cmd === "pet_scan") return Promise.resolve(scanOf([fourGroups[0]], 999)); // 图集也变了
      if (cmd === "pet_read_manifest") return Promise.resolve(manifest);
      return Promise.resolve(undefined);
    });
    const r = await activatePet("p1", async () => "ignore");
    expect(r.status).toBe("activated");
    expect(r.ignoredDiff).toBe(true); // 结构化标记（替代硬编码中文 message）
    expect(r.message).toBeUndefined();
    expect(localStorage.getItem("mam-pet-voice-cap")).toBe("0"); // done 缺失 → 无语音
  });

  it("manifest 不一致 + 忽略：manifest 条目全部在磁盘（含大小一致）→ voice-cap 保留 true", async () => {
    const manifest: PetManifestView = {
      id: "p1", displayName: "P", hasVoice: true, hasSubtitle: true,
      spriteVersionNumber: 2, spritesheetSizeBytes: 100,
      // FIX-7：条目大小须与磁盘一致（5 = fourGroups[0].size）才视为可信
      voices: [{ group: "general", name: "a", file: g("general"), sizeBytes: 5, durationMs: 3000 }],
    };
    tauriInvokeMock.mockImplementation((cmd: string) => {
      if (cmd === "pet_scan") return Promise.resolve(scanOf([fourGroups[0]], 999));
      if (cmd === "pet_read_manifest") return Promise.resolve(manifest);
      return Promise.resolve(undefined);
    });
    const r = await activatePet("p1", async () => "ignore");
    expect(r.status).toBe("activated");
    expect(r.ignoredDiff).toBe(true);
    expect(localStorage.getItem("mam-pet-voice-cap")).toBe("1"); // 条目齐全且大小一致 → 按 manifest 能力保留
  });

  it("manifest 不一致 + 忽略：文件存在但大小与 manifest 不一致 → voice-cap=0（即便条目覆盖四组，FIX-7）", async () => {
    // 磁盘四组文件都在（大小 5），manifest 记录 sizeBytes=99（大小已变）→ 缓存不可信 → 保守无语音
    const manifest: PetManifestView = {
      id: "p1", displayName: "P", hasVoice: true, hasSubtitle: true,
      spriteVersionNumber: 2, spritesheetSizeBytes: 100,
      voices: ["general", "approval", "done", "error"].map((n) => ({
        group: n, name: n, file: g(n), sizeBytes: 10, durationMs: 3000,
      })),
    };
    tauriInvokeMock.mockImplementation((cmd: string) => {
      if (cmd === "pet_scan") return Promise.resolve(scanOf(fourGroups, 999));
      if (cmd === "pet_read_manifest") return Promise.resolve(manifest);
      return Promise.resolve(undefined);
    });
    const r = await activatePet("p1", async () => "ignore");
    expect(r.status).toBe("activated");
    expect(r.ignoredDiff).toBe(true);
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