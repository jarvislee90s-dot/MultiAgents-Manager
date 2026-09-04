// 文件音效系统 — 12 个内置音效，全局默认 + 每工具覆盖（localStorage: mam-sound-config）

export interface SoundConfig {
  default: string; // 音效 id 或 "mute"
  tools: Partial<
    Record<"claude" | "codex" | "opencode" | "openclaw" | "kimi" | "workbuddy", string>
  >; // 音效 id 或 "mute"
}

export const SOUND_IDS = [
  "notification_accomplished_04",
  "notification_accomplished_06",
  "notification_activated_05",
  "notification_message_02",
  "notification_message_04",
  "notification_operation_failed_03",
  "notification_operation_succeed_01",
  "notification_operation_succeed_03",
  "notification_operation_succeed_06",
  "notification_operation_succeed_09",
  "notification_searching_03",
  "notification_wrong_02",
] as const;

const STORAGE_KEY = "mam-sound-config";
// 旧合成音配置键，读取时忽略并清理
const LEGACY_KEY = "mam-audio-frequencies";

export function getSoundConfig(): SoundConfig {
  try {
    localStorage.removeItem(LEGACY_KEY);
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved)
      return { default: "notification_operation_succeed_01", tools: {}, ...JSON.parse(saved) };
  } catch {
    // ignore
  }
  return { default: "notification_operation_succeed_01", tools: {} };
}

export function saveSoundConfig(config: SoundConfig) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(config));
}

// === 播放引擎（解码缓存） ===
let audioCtx: AudioContext | null = null;
const bufferCache = new Map<string, AudioBuffer>();

function getContext(): AudioContext | null {
  if (typeof window === "undefined") return null;
  if (!audioCtx) audioCtx = new AudioContext();
  if (audioCtx.state === "suspended") audioCtx.resume();
  return audioCtx;
}

async function loadBuffer(id: string): Promise<AudioBuffer | null> {
  if (bufferCache.has(id)) return bufferCache.get(id)!;
  try {
    const res = await fetch(`/sounds/${id}.wav`);
    const data = await res.arrayBuffer();
    const ctx = getContext();
    if (!ctx) return null;
    const buf = await ctx.decodeAudioData(data);
    bufferCache.set(id, buf);
    return buf;
  } catch {
    return null;
  }
}

/** 播放指定音效（试听与实际触发共用） */
export async function playSound(id: string) {
  if (!SOUND_IDS.includes(id as (typeof SOUND_IDS)[number])) return;
  const ctx = getContext();
  const buf = await loadBuffer(id);
  if (!ctx || !buf) return;
  const source = ctx.createBufferSource();
  source.buffer = buf;
  source.connect(ctx.destination);
  source.start();
}

/** 任务完成（→绿）时按工具播放：专属覆盖 → 全局默认；mute 跳过 */
export function playCompletionSound(agentType: string) {
  const cfg = getSoundConfig();
  const id = cfg.tools[agentType as keyof SoundConfig["tools"]] ?? cfg.default;
  if (id && id !== "mute") playSound(id);
}
