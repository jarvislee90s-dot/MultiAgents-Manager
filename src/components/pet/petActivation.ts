// 激活编排 — 统一校验算法交互层：直投生成 / 不一致修复 / 忽略降级 / 激活指针（spec §6）
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { PetError } from "./petErrors";
import { probeAudioDurationMs, probeSheetRows, saveActiveId } from "./petRuntime";
import {
  diffManifestVsScan,
  isAudioCandidate,
  judgeVoiceTier,
  spriteVersionOf,
  type PetManifestView,
  type PetScan,
  type ValidationIssue,
} from "./petValidation";

export type MismatchChoice = "update" | "ignore" | "cancel";
export type MismatchConfirm = (
  issues: ValidationIssue[],
  manifest: PetManifestView
) => Promise<MismatchChoice>;

export interface ActivationResult {
  status: "activated" | "invalid-sheet" | "mismatch" | "error";
  manifestBuilt?: boolean;
  repaired?: boolean;
  /** ignore 降级激活标记（UI 据此用 ignoredDiff 文案 toast，FIX-3） */
  ignoredDiff?: boolean;
  /** 原始异常（PetError/RpcError/普通 Error），展示层经 petErrMsg 翻译（P3-1） */
  err?: unknown;
  issues?: ValidationIssue[];
}

function notifyPetChanged(): void {
  emit("pet-active-changed", {}).catch(() => {});
}

/** 探测扫描中的全部合法候选（并行，失败项 durationMs=null） */
async function probeCandidates(
  scan: PetScan
): Promise<{ rel: string; size: number; durationMs: number | null }[]> {
  const candidates = scan.voiceFiles.filter(isAudioCandidate);
  return Promise.all(
    candidates.map(async (f) => ({
      rel: f.rel,
      size: f.size,
      durationMs: await probeAudioDurationMs(convertFileSrc(`${scan.dir}/${f.rel}`)).catch(() => null),
    }))
  );
}

const isValid = (p: { size: number; durationMs: number | null }) =>
  p.durationMs !== null &&
  p.durationMs > 1000 &&
  p.durationMs < 20000 &&
  p.size <= 10 * 1024 * 1024;

/** 直投首激活：全量探测 → 三档判定 → 生成 manifest（spec §6-2） */
export async function buildManifestFromScan(
  id: string,
  scan: PetScan,
  rows: 9 | 11,
  source: string,
  subtitleDefault: boolean,
  overrides?: { displayName?: string; description?: string }
): Promise<PetManifestView> {
  const probed = await probeCandidates(scan);
  const valid = probed.filter(isValid);
  const hasVoice = judgeVoiceTier(valid).hasVoice;
  return {
    id,
    displayName: overrides?.displayName || id,
    description: overrides?.description || "",
    source,
    hasVoice,
    hasSubtitle: hasVoice && subtitleDefault,
    spriteVersionNumber: spriteVersionOf(rows),
    spritesheetSizeBytes: scan.spritesheet.size,
    voices: valid.map((p) => ({
      group: p.rel.split("/")[1],
      name: p.rel.split("/").pop()!.replace(/\.[^.]+$/, ""),
      file: p.rel,
      sizeBytes: p.size,
      durationMs: p.durationMs!,
    })),
  };
}

/** 修复：保留未变条目（信任缓存时长）、重探变动与新增（spec §6-3） */
export async function repairManifest(
  old: PetManifestView,
  scan: PetScan,
  rows: 9 | 11
): Promise<PetManifestView> {
  const keep = old.voices.filter(
    (v) => scan.voiceFiles.some((f) => f.rel === v.file && f.size === v.sizeBytes)
  );
  const changedOrNew = scan.voiceFiles
    .filter(isAudioCandidate)
    .filter((f) => !keep.some((v) => v.file === f.rel));
  const probed = await Promise.all(
    changedOrNew.map(async (f) => ({
      rel: f.rel,
      size: f.size,
      durationMs: await probeAudioDurationMs(convertFileSrc(`${scan.dir}/${f.rel}`)).catch(() => null),
    }))
  );
  const validNew = probed.filter(isValid).map((p) => ({
    group: p.rel.split("/")[1],
    name: p.rel.split("/").pop()!.replace(/\.[^.]+$/, ""),
    file: p.rel,
    sizeBytes: p.size,
    durationMs: p.durationMs!,
  }));
  const voices = [...keep, ...validNew];
  const hasVoice = judgeVoiceTier(voices.map((v) => ({ rel: v.file, size: v.sizeBytes, durationMs: v.durationMs }))).hasVoice;
  return {
    ...old,
    spriteVersionNumber: spriteVersionOf(rows),
    spritesheetSizeBytes: scan.spritesheet.size,
    hasVoice,
    hasSubtitle: hasVoice && old.hasSubtitle,
    voices,
  };
}

/** 统一激活入口（切换/启动修复共用，spec §6） */
export async function activatePet(id: string, confirm: MismatchConfirm): Promise<ActivationResult> {
  try {
    if (id === "foxbell") {
      saveActiveId("foxbell", true, "Foxbell");
      notifyPetChanged();
      return { status: "activated" };
    }
    const scan = await invoke<PetScan>("pet_scan", { id });
    if (!scan.spritesheet.exists) {
      return { status: "invalid-sheet", err: new PetError("sheet-missing") };
    }
    let rows: 9 | 11;
    try {
      rows = await probeSheetRows(convertFileSrc(`${scan.dir}/spritesheet.webp`));
    } catch (e) {
      return { status: "invalid-sheet", err: e };
    }
    const manifest = await invoke<PetManifestView | null>("pet_read_manifest", { id });
    if (!manifest) {
      const built = await buildManifestFromScan(id, scan, rows, "folder", true);
      await invoke("pet_update_manifest", { id, manifest: built, backup: false });
      saveActiveId(id, built.hasVoice, built.displayName);
      notifyPetChanged();
      return { status: "activated", manifestBuilt: true };
    }
    const issues = diffManifestVsScan(manifest, scan);
    if (issues.length === 0) {
      saveActiveId(id, manifest.hasVoice, manifest.displayName);
      notifyPetChanged();
      return { status: "activated" };
    }
    const choice = await confirm(issues, manifest);
    if (choice === "cancel") {
      return { status: "mismatch", issues };
    }
    if (choice === "update") {
      const repaired = await repairManifest(manifest, scan, rows);
      await invoke("pet_update_manifest", { id, manifest: repaired, backup: true });
      saveActiveId(id, repaired.hasVoice, repaired.displayName);
      notifyPetChanged();
      return { status: "activated", repaired: true };
    }
    // ignore：按磁盘现状运行，不重写 manifest（FIX-3 诚实语义）。
    // voice-cap 按 manifest 语音条目与磁盘的比对判定（FIX-7）：存在且大小与缓存一致 → 可信；
    // 任一缺失或大小已变 → 缓存时长失效（spec §4.2 缓存语义前提被破坏），保守无语音（spec §6-3）
    const manifestEntriesOnDisk =
      manifest.voices.length > 0 &&
      manifest.voices.every((v) =>
        scan.voiceFiles.some((f) => f.rel === v.file && f.exists && f.size === v.sizeBytes)
      );
    const voiceCap = manifestEntriesOnDisk ? manifest.hasVoice : false;
    saveActiveId(id, voiceCap, manifest.displayName);
    notifyPetChanged();
    return { status: "activated", ignoredDiff: true };
  } catch (e) {
    return { status: "error", err: e };
  }
}