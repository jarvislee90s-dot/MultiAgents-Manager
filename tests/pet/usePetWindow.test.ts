// tests/pet/usePetWindow.test.ts
import { describe, expect, it } from "vitest";
import { bottomAnchoredY, clampToWorkArea, hitTest, stepFall, GRAVITY } from "@/components/pet/usePetWindow";

describe("usePetWindow pure helpers", () => {
  it("bottomAnchoredY：新高度下保持底边不动（spec §4.2）", () => {
    expect(bottomAnchoredY(500, 260, 360)).toBe(400); // 500+260-360
    expect(bottomAnchoredY(500, 260, 620)).toBe(140);
  });

  it("clampToWorkArea：越界夹紧", () => {
    const work = { x: 0, y: 40, width: 1000, height: 960 };
    expect(clampToWorkArea(-50, 0, 340, 260, work)).toEqual({ x: 0, y: 40 });
    expect(clampToWorkArea(900, 900, 340, 260, work)).toEqual({ x: 660, y: 740 });
    expect(clampToWorkArea(100, 100, 340, 260, work)).toEqual({ x: 100, y: 100 });
  });

  it("hitTest：点在任一矩形内", () => {
    const rects = [{ x: 10, y: 10, w: 100, h: 50 }];
    expect(hitTest(rects, 50, 30)).toBe(true);
    expect(hitTest(rects, 5, 30)).toBe(false);
    expect(hitTest([], 50, 30)).toBe(false);
  });

  it("stepFall：重力加速、阻尼衰减、落地判定、静止阈值（spec §8）", () => {
    let s = { x: 0, y: 0, vx: 500, vy: 0 };
    let landed = false;
    for (let i = 0; i < 300 && !s.rest; i++) {
      const r = stepFall(s, 1 / 60, 700);
      s = { x: r.x, y: r.y, vx: r.vx, vy: r.vy };
      landed = landed || r.landed;
    }
    expect(landed).toBe(true);
    expect(s.y).toBe(700);
    expect(s.vx).toBeLessThan(24); // 阻尼后静止
    // 无初速垂直坠落 0.5s：y ≈ ½gt²
    const f = stepFall({ x: 0, y: 0, vx: 0, vy: 0 }, 0.5, 100000);
    expect(Math.abs(f.y - (0.5 * GRAVITY * 0.25))).toBeLessThan(1e-6);
  });
});
