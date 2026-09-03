// 激活宠物运行时 — 指针持久化、foxbell 描述符、外部宠物解析、媒体探测与音频内存快照（spec §7/§12）
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { parseManifest, type VoiceEntry } from "./petVoices";

export const ACTIVE_KEY = "mam-pet-active";
export const ACTIVE_NAME_KEY = "mam-pet-active-name";
export const VOICE_CAP_KEY = "mam-pet-voice-cap";

export type PetRows = 9 | 11;

export interface ActivePet {
  id: string; // "foxbell" | 外部宠物 ID
  displayName: string;
  spritesheetUrl: string;
  rows: PetRows;
  hasVoice: boolean;
  hasSubtitle: boolean;
  voices: VoiceEntry[];
  resolveVoiceUrl: (file: string) => string;
  /** 释放运行时资源（外部宠物 = 逐个 revoke 音频 blob URL）；foxbell 无快照故无 dispose */
  dispose?: () => void;
}

/** 内置 foxbell 描述符（voices 由 manifest.json 拉取后填充，spec EP10） */
export const FOXBELL: ActivePet = {
  id: "foxbell",
  displayName: "Foxbell",
  spritesheetUrl: "/pet/spritesheet.webp",
  rows: 11,
  hasVoice: true,
  hasSubtitle: true,
  voices: [],
  resolveVoiceUrl: (f) => `/pet/voice/${encodeURI(f)}`,
};

export function loadActiveId(): string {
  try {
    return localStorage.getItem(ACTIVE_KEY) || "foxbell";
  } catch {
    return "foxbell";
  }
}

export function loadActiveName(): string {
  try {
    return localStorage.getItem(ACTIVE_NAME_KEY) || "Foxbell";
  } catch {
    return "Foxbell";
  }
}

/** 激活指针 + 语音能力缓存 + 展示名缓存（petSoundTakeover 同步读取用，spec §5.2） */
export function saveActiveId(id: string, voiceCap: boolean, displayName?: string): void {
  localStorage.setItem(ACTIVE_KEY, id);
  localStorage.setItem(VOICE_CAP_KEY, voiceCap ? "1" : "0");
  if (displayName) localStorage.setItem(ACTIVE_NAME_KEY, displayName);
}

/** 语音能力：未写入时视为 true（foxbell / 旧版本升级兼容） */
export function loadVoiceCap(): boolean {
  try {
    return localStorage.getItem(VOICE_CAP_KEY) !== "0";
  } catch {
    return true;
  }
}

/** 图集尺寸 → 行数（1536×1872→9，1536×2288→11，其余非法） */
export function rowsFromSize(w: number, h: number): PetRows | null {
  if (w !== 1536) return null;
  if (h === 1872) return 9;
  if (h === 2288) return 11;
  return null;
}

/** 图集行数探测（Image 解码，约 50-150ms） */
export function probeSheetRows(url: string): Promise<PetRows> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => {
      const r = rowsFromSize(img.naturalWidth, img.naturalHeight);
      if (r) resolve(r);
      else reject(new Error(`spritesheet 尺寸非法: ${img.naturalWidth}x${img.naturalHeight}`));
    };
    img.onerror = () => reject(new Error("spritesheet 加载失败"));
    img.src = url;
  });
}

/** 音频时长探测（仅读元数据头部，超时 8s，spec §6-2） */
export function probeAudioDurationMs(url: string, timeoutMs = 8000): Promise<number> {
  return new Promise((resolve, reject) => {
    const a = new Audio();
    a.preload = "metadata";
    const timer = window.setTimeout(() => {
      a.src = "";
      reject(new Error("音频探测超时"));
    }, timeoutMs);
    a.onloadedmetadata = () => {
      window.clearTimeout(timer);
      const d = a.duration;
      a.src = "";
      if (Number.isFinite(d) && d > 0) resolve(Math.round(d * 1000));
      else reject(new Error("音频时长不可用"));
    };
    a.onerror = () => {
      window.clearTimeout(timer);
      reject(new Error("音频加载失败"));
    };
    a.src = url;
  });
}

interface ScanFile {
  rel: string;
  exists: boolean;
  size: number;
}
interface PetScanDto {
  id: string;
  dir: string;
  spritesheet: ScanFile;
  voiceFiles: ScanFile[];
}
interface ManifestVoiceDto {
  group: string;
  name: string;
  file: string;
  sizeBytes: number;
  durationMs: number;
}
interface ManifestDto {
  id: string;
  displayName: string;
  hasVoice: boolean;
  hasSubtitle: boolean;
  spriteVersionNumber: number;
  voices: ManifestVoiceDto[];
}

/** 音频内存快照：激活时全量 fetch → blob URL（EP6；任一失败整体降级为无语音）。
 *  返回的 dispose 逐个 revoke 创建的 objectURL（FIX-2：切换/卸载时防泄漏） */
async function snapshotVoices(
  dir: string,
  voices: ManifestVoiceDto[]
): Promise<{ entries: VoiceEntry[]; resolve: (file: string) => string; dispose: () => void } | null> {
  const blobs = new Map<string, string>();
  const created: string[] = [];
  try {
    await Promise.all(
      voices.map(async (v) => {
        const res = await fetch(convertFileSrc(`${dir}/${v.file}`));
        if (!res.ok) throw new Error(`快照失败: ${v.file}`);
        const url = URL.createObjectURL(await res.blob());
        created.push(url);
        blobs.set(v.file, url);
      })
    );
    return {
      // index 必须顺序编号：VoicePlayer.play 以 els[entry.index] 定位预载元素
      entries: voices.map((v, i) => ({
        index: i,
        group: v.group as VoiceEntry["group"],
        name: v.name,
        file: v.file,
      })),
      resolve: (file) => blobs.get(file) ?? "",
      dispose: () => {
        for (const url of created) {
          try {
            URL.revokeObjectURL(url);
          } catch {
            /* ignore */
          }
        }
        created.length = 0;
      },
    };
  } catch {
    // 快照中途失败：撤销已创建的 URL，不留泄漏
    for (const url of created) {
      try {
        URL.revokeObjectURL(url);
      } catch {
        /* ignore */
      }
    }
    return null;
  }
}

/**
 * 解析当前激活宠物（宠物窗口启动 / 热切换共用）。
 * foxbell：静态描述符 + manifest.json 语音；外部：scan + manifest + 图集探测 + 音频快照。
 * 任何失败抛错，调用方回落 FOXBELL（spec §5.2 宠物永不白屏）。
 */
export async function resolveActivePet(): Promise<ActivePet> {
  const id = loadActiveId();
  if (id === "foxbell") {
    try {
      const raw = await fetch("/pet/manifest.json").then((r) => r.json());
      return { ...FOXBELL, voices: parseManifest(raw) };
    } catch {
      return FOXBELL; // 浏览器渲染/素材缺失：静默降级
    }
  }
  const scan = await invoke<PetScanDto>("pet_scan", { id });
  if (!scan.spritesheet.exists) throw new Error("spritesheet.webp 缺失");
  const rows = await probeSheetRows(convertFileSrc(`${scan.dir}/spritesheet.webp`));
  const manifest = await invoke<ManifestDto | null>("pet_read_manifest", { id });
  if (!manifest) {
    // 直投未生成 manifest：渲染可用的最低档（主窗口启动校验负责生成，spec §6.1）
    return {
      id,
      displayName: id,
      spritesheetUrl: convertFileSrc(`${scan.dir}/spritesheet.webp`),
      rows,
      hasVoice: false,
      hasSubtitle: false,
      voices: [],
      resolveVoiceUrl: () => "",
    };
  }
  const snap = manifest.hasVoice ? await snapshotVoices(scan.dir, manifest.voices) : null;
  return {
    id,
    displayName: manifest.displayName || id,
    spritesheetUrl: convertFileSrc(`${scan.dir}/spritesheet.webp`),
    rows,
    hasVoice: !!snap,
    hasSubtitle: manifest.hasSubtitle && !!snap,
    voices: snap?.entries ?? [],
    resolveVoiceUrl: snap?.resolve ?? (() => ""),
    dispose: snap?.dispose,
  };
}