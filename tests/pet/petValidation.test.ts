import { describe, expect, it } from "vitest";
import {
  AUDIO_EXTS, MAX_AUDIO_BYTES, MIN_DURATION_MS, MAX_DURATION_MS,
  groupOfRel, nameFromRel, isAudioCandidate, diffManifestVsScan, judgeVoiceTier, voiceRowProblem,
  type PetScan, type PetManifestView,
} from "@/components/pet/petValidation";

const scan = (files: { rel: string; size: number }[], sheetSize = 100): PetScan => ({
  id: "p",
  dir: "/x/p",
  spritesheet: { rel: "spritesheet.webp", exists: sheetSize > 0, size: sheetSize },
  voiceFiles: files.map((f) => ({ rel: f.rel, exists: true, size: f.size })),
});

describe("petValidation", () => {
  it("分组与文件名解析", () => {
    expect(groupOfRel("voice/general/a.m4a")).toBe("general");
    expect(groupOfRel("voice/hack/a.m4a")).toBeNull();
    expect(groupOfRel("a.m4a")).toBeNull();
    expect(nameFromRel("voice/general/休息一下吧.m4a")).toBe("休息一下吧");
  });

  it("合法音频候选：扩展名 + 分组（spec §5.1）", () => {
    expect(isAudioCandidate({ rel: "voice/done/x.MP3", exists: true, size: 1 })).toBe(true);
    expect(isAudioCandidate({ rel: "voice/done/x.txt", exists: true, size: 1 })).toBe(false);
    expect(isAudioCandidate({ rel: "other/x.mp3", exists: true, size: 1 })).toBe(false);
    expect(AUDIO_EXTS).toContain("m4a");
    expect(MAX_AUDIO_BYTES).toBe(10 * 1024 * 1024);
    expect(MIN_DURATION_MS).toBe(1000);
    expect(MAX_DURATION_MS).toBe(20000);
  });

  it("voiceRowProblem：时长/大小边界（spec §5.1 严格不等）", () => {
    const ok = { group: "general", name: "a", file: "voice/general/a.m4a", sizeBytes: 1, durationMs: 2000 };
    expect(voiceRowProblem(ok)).toBeNull();
    expect(voiceRowProblem({ ...ok, durationMs: 1000 })).toBe("too-short");
    expect(voiceRowProblem({ ...ok, durationMs: 20000 })).toBe("too-long");
    expect(voiceRowProblem({ ...ok, durationMs: null })).toBe("no-duration");
    expect(voiceRowProblem({ ...ok, sizeBytes: MAX_AUDIO_BYTES + 1 })).toBe("too-big");
  });

  it("judgeVoiceTier：四组各≥1 合法才开语音（全有或全无）", () => {
    const v = { rel: "", size: 1, durationMs: 2000 };
    const mk = (g: string) => ({ ...v, rel: `voice/${g}/a.m4a` });
    expect(judgeVoiceTier([mk("general"), mk("approval"), mk("done"), mk("error")]).hasVoice).toBe(true);
    expect(judgeVoiceTier([mk("general"), mk("approval"), mk("done")]).hasVoice).toBe(false);
    expect(judgeVoiceTier([]).hasVoice).toBe(false);
    // 单组不合法即整组无覆盖
    expect(
      judgeVoiceTier([mk("general"), mk("approval"), mk("done"), { ...mk("error"), durationMs: 25000 }]).hasVoice
    ).toBe(false);
  });

  it("diffManifestVsScan：一致无 issue；缺文件/大小变/多余文件/图集变（spec §6-3）", () => {
    const m: PetManifestView = {
      id: "p", displayName: "P", hasVoice: true, hasSubtitle: true,
      spriteVersionNumber: 2, spritesheetSizeBytes: 100,
      voices: [{ group: "general", name: "a", file: "voice/general/a.m4a", sizeBytes: 10, durationMs: 2000 }],
    };
    expect(diffManifestVsScan(m, scan([{ rel: "voice/general/a.m4a", size: 10 }]))).toEqual([]);
    const issues = diffManifestVsScan(
      m,
      scan([{ rel: "voice/general/a.m4a", size: 99 }, { rel: "voice/done/new.mp3", size: 5 }], 999)
    );
    expect(issues.map((i) => i.kind)).toEqual(["spritesheet-changed", "voice-changed", "voice-extra"]);
    const missing = diffManifestVsScan(m, scan([], 100));
    expect(missing.map((i) => i.kind)).toContain("voice-missing");
    const noSheet = diffManifestVsScan(m, scan([{ rel: "voice/general/a.m4a", size: 10 }], 0));
    expect(noSheet.map((i) => i.kind)).toContain("spritesheet-missing");
  });
});