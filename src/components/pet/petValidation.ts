// 统一校验纯函数 — manifest×磁盘 diff、音频合法性、三档判定（spec §6；探测由 petRuntime 提供）
export const GROUPS = ["general", "approval", "done", "error"] as const;
export const AUDIO_EXTS = ["m4a", "mp3", "wav", "ogg", "opus", "flac", "aac"];
export const MAX_AUDIO_BYTES = 10 * 1024 * 1024;
export const MIN_DURATION_MS = 1000;
export const MAX_DURATION_MS = 20000;

export interface ScanFile {
  rel: string;
  exists: boolean;
  size: number;
}

export interface PetScan {
  id: string;
  dir: string;
  spritesheet: ScanFile;
  voiceFiles: ScanFile[];
}

export interface ManifestVoice {
  group: string;
  name: string;
  file: string;
  sizeBytes: number;
  durationMs: number;
}

export interface PetManifestView {
  id: string;
  displayName: string;
  description?: string;
  source?: string;
  hasVoice: boolean;
  hasSubtitle: boolean;
  spriteVersionNumber: number;
  spritesheetSizeBytes: number;
  voices: ManifestVoice[];
}

export type VoiceProblem = "too-short" | "too-long" | "too-big" | "no-duration";

export interface VoiceRow {
  group: string;
  name: string;
  file: string;
  sizeBytes: number;
  durationMs: number | null;
}

export function extOf(rel: string): string {
  const i = rel.lastIndexOf(".");
  return i >= 0 ? rel.slice(i + 1).toLowerCase() : "";
}

/** "voice/<group>/<file>" → group（仅四固定分组） */
export function groupOfRel(rel: string): string | null {
  const parts = rel.split("/");
  if (parts.length !== 3 || parts[0] !== "voice") return null;
  return (GROUPS as readonly string[]).includes(parts[1]) ? parts[1] : null;
}

/** 字幕文本 = 文件名去扩展名（EP8） */
export function nameFromRel(rel: string): string {
  const base = rel.split("/").pop() ?? rel;
  const i = base.lastIndexOf(".");
  return i > 0 ? base.slice(0, i) : base;
}

/** 扫描项是否为合法音频候选（扩展名 + 分组路径，spec §5.1） */
export function isAudioCandidate(f: ScanFile): boolean {
  return f.exists && AUDIO_EXTS.includes(extOf(f.rel)) && groupOfRel(f.rel) !== null;
}

/** 单音频合法性（探测后判定；durationMs=null 表示探测失败） */
export function voiceRowProblem(r: VoiceRow): VoiceProblem | null {
  if (r.durationMs === null) return "no-duration";
  if (r.durationMs <= MIN_DURATION_MS) return "too-short";
  if (r.durationMs >= MAX_DURATION_MS) return "too-long";
  if (r.sizeBytes > MAX_AUDIO_BYTES) return "too-big";
  return null;
}

export interface TierJudge {
  hasVoice: boolean;
  coverage: Record<string, number>;
}

/** 声音档判定：四组各 ≥1 合法文件（全有或全无，spec §5.1） */
export function judgeVoiceTier(
  files: { rel: string; size: number; durationMs: number | null }[]
): TierJudge {
  const coverage: Record<string, number> = {};
  for (const g of GROUPS) coverage[g] = 0;
  for (const f of files) {
    if (
      voiceRowProblem({
        group: "",
        name: "",
        file: f.rel,
        sizeBytes: f.size,
        durationMs: f.durationMs,
      })
    )
      continue;
    const g = groupOfRel(f.rel);
    if (g) coverage[g] += 1;
  }
  return { hasVoice: GROUPS.every((g) => coverage[g] > 0), coverage };
}

export type IssueKind =
  | "spritesheet-missing"
  | "spritesheet-changed"
  | "voice-missing"
  | "voice-changed"
  | "voice-extra"
  | "manifest-missing";

export interface ValidationIssue {
  kind: IssueKind;
  /** 语言中性数据（路径/纯数字），展示标签由 pet.issue.<kind> 提供（P3-6） */
  detail: string;
}

/** manifest × 磁盘 stat 比对（不解码媒体，spec §6-3） */
export function diffManifestVsScan(m: PetManifestView, s: PetScan): ValidationIssue[] {
  const issues: ValidationIssue[] = [];
  if (!s.spritesheet.exists) {
    issues.push({ kind: "spritesheet-missing", detail: "spritesheet.webp" });
  } else if (m.spritesheetSizeBytes > 0 && s.spritesheet.size !== m.spritesheetSizeBytes) {
    issues.push({
      kind: "spritesheet-changed",
      detail: `${m.spritesheetSizeBytes} → ${s.spritesheet.size}`,
    });
  }
  const onDisk = new Map(s.voiceFiles.map((f) => [f.rel, f.size]));
  for (const v of m.voices) {
    if (!onDisk.has(v.file)) issues.push({ kind: "voice-missing", detail: v.file });
    else if (onDisk.get(v.file) !== v.sizeBytes)
      issues.push({ kind: "voice-changed", detail: v.file });
  }
  const known = new Set(m.voices.map((v) => v.file));
  for (const f of s.voiceFiles) {
    if (isAudioCandidate(f) && !known.has(f.rel))
      issues.push({ kind: "voice-extra", detail: f.rel });
  }
  return issues;
}

/** rows ↔ spriteVersionNumber */
export function spriteVersionOf(rows: 9 | 11): 1 | 2 {
  return rows === 9 ? 1 : 2;
}

// ---- 宠物 id 前端实时校验（spec §8.4-1/§10-1，issue #33-7）----
// 规则与后端 validate_pet_id 对齐（issue #32-1）：字符集 + 保留设备名 + 长度；重名为前端预检

/** Windows 保留设备名（大小写不敏感），与 src-tauri validate_pet_id 同表 */
const RESERVED_DEVICE_NAMES = new Set([
  "CON",
  "PRN",
  "AUX",
  "NUL",
  ...Array.from({ length: 9 }, (_, i) => `COM${i + 1}`),
  ...Array.from({ length: 9 }, (_, i) => `LPT${i + 1}`),
]);

export const PET_NAME_MAX_LEN = 64;

export type PetNameProblem = "charset" | "too-long" | "reserved-device" | "duplicate";

/** 宠物 id 实时校验：返回首个问题，null = 合法。空串由调用方按「按钮禁用」处理；
 *  selfId 用于重命名场景（保持自身名不算重名） */
export function petNameProblem(
  name: string,
  opts: { existingIds?: string[]; selfId?: string; maxLength?: number } = {}
): PetNameProblem | null {
  const max = opts.maxLength ?? PET_NAME_MAX_LEN;
  if (name.length > max) return "too-long";
  if (!/^[A-Za-z0-9_-]+$/.test(name)) return "charset";
  if (RESERVED_DEVICE_NAMES.has(name.toUpperCase())) return "reserved-device";
  const others = (opts.existingIds ?? []).filter((id) => id !== opts.selfId);
  if (others.some((id) => id.toLowerCase() === name.toLowerCase())) return "duplicate";
  return null;
}

/** PetNameProblem → i18n 键（charset 复用通用 nameHint 文案），导入/重命名 UI 共用（issue #33-7） */
export function petNameProblemKey(p: PetNameProblem): string {
  switch (p) {
    case "duplicate":
      return "pet.import.nameDup";
    case "reserved-device":
      return "pet.import.nameReserved";
    case "too-long":
      return "pet.import.nameTooLong";
    case "charset":
      return "pet.import.nameHint";
  }
}

/** manifest 语音条目在磁盘的可信度（spec §4.2 缓存语义）：全部条目存在且大小一致 → hasVoice
 *  可信，任一缺失/大小已变 → 缓存前提被破坏，保守无语音（spec §6-3）。
 *  ignore 降级激活与重命名切回共用同一判定（issue #33-8/#33-12） */
export function manifestVoiceCapOnDisk(m: PetManifestView, s: PetScan): boolean {
  const onDisk =
    m.voices.length > 0 &&
    m.voices.every((v) =>
      s.voiceFiles.some((f) => f.rel === v.file && f.exists && f.size === v.sizeBytes)
    );
  return onDisk ? m.hasVoice : false;
}
