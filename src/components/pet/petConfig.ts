// 桌宠配置 — localStorage 单后端，跨窗口 storage 事件同步（spec §10）
import { loadVoiceCap } from "./petRuntime";

export type PetAction = "jumping" | "waving" | "failed" | "waiting" | "review" | "running";
export type PetScale = 0.75 | 1 | 1.25;

export interface PetConfig {
  alwaysOnTop: boolean;
  muted: boolean;
  talkative: boolean;
  gravity: boolean;
  scale: PetScale;
  dblAction: PetAction;
  approvalAction: PetAction;
  errorAction: PetAction; // 占位：v1 无触发场景（spec D9/D14）
  doneAction: PetAction;
}

export const CONFIG_KEY = "mam-pet-config";
export const VISIBLE_KEY = "mam-pet-visible";
export const POSITION_KEY = "mam-pet-position";

export const PET_ACTIONS: PetAction[] = [
  "jumping",
  "waving",
  "failed",
  "waiting",
  "review",
  "running",
];
export const PET_SCALES: PetScale[] = [0.75, 1, 1.25];

const DEFAULT_CONFIG: PetConfig = {
  alwaysOnTop: true,
  muted: false,
  talkative: true,
  gravity: true,
  scale: 1,
  dblAction: "waving",
  approvalAction: "waiting",
  errorAction: "failed",
  doneAction: "jumping",
};

const ACTION_KEYS = ["dblAction", "approvalAction", "errorAction", "doneAction"] as const;
const BOOL_KEYS = ["alwaysOnTop", "muted", "talkative", "gravity"] as const;

function sanitize(raw: unknown): PetConfig {
  const out = { ...DEFAULT_CONFIG };
  if (raw && typeof raw === "object") {
    const p = raw as Record<string, unknown>;
    for (const k of BOOL_KEYS) if (typeof p[k] === "boolean") out[k] = p[k] as boolean;
    if (PET_SCALES.includes(p.scale as PetScale)) out.scale = p.scale as PetScale;
    for (const k of ACTION_KEYS)
      if (PET_ACTIONS.includes(p[k] as PetAction)) out[k] = p[k] as PetAction;
  }
  return out;
}

export function loadConfig(): PetConfig {
  try {
    const raw = localStorage.getItem(CONFIG_KEY);
    return raw ? sanitize(JSON.parse(raw)) : { ...DEFAULT_CONFIG };
  } catch {
    return { ...DEFAULT_CONFIG };
  }
}

const listeners = new Set<() => void>();
const emit = () => listeners.forEach((fn) => fn());

export function saveConfig(patch: Partial<PetConfig>): void {
  localStorage.setItem(CONFIG_KEY, JSON.stringify(sanitize({ ...loadConfig(), ...patch })));
  emit();
}

export function subscribeConfig(fn: () => void): () => void {
  listeners.add(fn);
  // storage 事件只在"其它窗口"修改时触发；本窗口修改靠 emit
  const onStorage = (e: StorageEvent) => {
    if (e.key === null || e.key === CONFIG_KEY || e.key === VISIBLE_KEY) fn();
  };
  window.addEventListener("storage", onStorage);
  return () => {
    listeners.delete(fn);
    window.removeEventListener("storage", onStorage);
  };
}

export function loadVisible(): boolean {
  try {
    return localStorage.getItem(VISIBLE_KEY) === "1";
  } catch {
    return false;
  }
}

export function saveVisible(v: boolean): void {
  localStorage.setItem(VISIBLE_KEY, v ? "1" : "0");
  emit();
}

export interface PetPosition {
  x: number;
  y: number;
}

export function loadPosition(): PetPosition | null {
  try {
    const raw = localStorage.getItem(POSITION_KEY);
    if (!raw) return null;
    const p = JSON.parse(raw);
    if (Number.isFinite(p?.x) && Number.isFinite(p?.y)) return { x: p.x, y: p.y };
  } catch {
    // ignore
  }
  return null;
}

export function savePosition(pos: PetPosition): void {
  localStorage.setItem(
    POSITION_KEY,
    JSON.stringify({ x: Math.round(pos.x), y: Math.round(pos.y) })
  );
}

/** 完成提示音接管：宠物开启且当前宠物具备语音能力（无语音外部宠物回落主看板，spec §5.2） */
export function petSoundTakeover(): boolean {
  return loadVisible() && loadVoiceCap();
}

/** 通知浮窗抑制：宠物开启且置顶（spec D4） */
export function petSuppressPopup(): boolean {
  return loadVisible() && loadConfig().alwaysOnTop;
}
