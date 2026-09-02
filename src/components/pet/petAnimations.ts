// 精灵动画 — Codex V2 图集（8 列×11 行，每帧 192×208），逐帧时长 + look 16 向（spec §7）
export const FRAME_W = 192;
export const FRAME_H = 208;
export const SHEET_COLS = 8;
export const SHEET_ROWS = 11;

export type PetAnimKey =
  | "idle"
  | "run-right"
  | "run-left"
  | "waving"
  | "jumping"
  | "failed"
  | "waiting"
  | "running"
  | "review"
  | "look";

export const ANIM: Record<Exclude<PetAnimKey, "look">, { row: number; d: number[] }> = {
  idle: { row: 0, d: [280, 110, 110, 140, 140, 320] },
  "run-right": { row: 1, d: [120, 120, 120, 120, 120, 120, 120, 220] },
  "run-left": { row: 2, d: [120, 120, 120, 120, 120, 120, 120, 220] },
  waving: { row: 3, d: [140, 140, 140, 280] },
  jumping: { row: 4, d: [140, 140, 140, 140, 280] },
  failed: { row: 5, d: [140, 140, 140, 140, 140, 140, 140, 240] },
  waiting: { row: 6, d: [150, 150, 150, 150, 150, 260] },
  running: { row: 7, d: [120, 120, 120, 120, 120, 220] },
  review: { row: 8, d: [150, 150, 150, 150, 150, 280] },
};

// look 行 9→10：16 向顺时针连续扫视（行9 列0..7 → 行10 列0..7）
// 注意用 0 - col*W 而非 -(col)*W：i=0 时后者会产生 -0（vitest toEqual 区分 ±0）
export const LOOK_FRAMES = Array.from({ length: 16 }, (_, i) => ({
  x: 0 - (i % SHEET_COLS) * FRAME_W,
  y: 0 - (i < 8 ? 9 : 10) * FRAME_H,
}));

export function frameStyle(
  anim: PetAnimKey,
  frame: number,
  lookFrame: number,
  scale: number
): { backgroundPosition: string; backgroundSize: string } {
  const w = FRAME_W * scale;
  const h = FRAME_H * scale;
  let x: number;
  let y: number;
  if (anim === "look") {
    const f = LOOK_FRAMES[Math.max(0, Math.min(LOOK_FRAMES.length - 1, lookFrame))];
    x = f.x * scale;
    y = f.y * scale;
  } else {
    const def = ANIM[anim];
    const i = ((frame % def.d.length) + def.d.length) % def.d.length;
    x = -i * w;
    y = -def.row * h;
  }
  return {
    backgroundPosition: `${x}px ${y}px`,
    backgroundSize: `${w * SHEET_COLS}px ${h * SHEET_ROWS}px`,
  };
}
