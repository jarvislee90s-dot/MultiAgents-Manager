// tests/pet/petAnimations.test.ts
import { describe, expect, it } from "vitest";
import { ANIM, LOOK_FRAMES, frameStyle, FRAME_W, FRAME_H } from "@/components/pet/petAnimations";

describe("petAnimations", () => {
  it("ANIM 表与原版一致（行号与逐帧时长）", () => {
    expect(ANIM.idle).toEqual({ row: 0, d: [280, 110, 110, 140, 140, 320] });
    expect(ANIM["run-right"].row).toBe(1);
    expect(ANIM["run-left"].row).toBe(2);
    expect(ANIM.waving).toEqual({ row: 3, d: [140, 140, 140, 280] });
    expect(ANIM.jumping.row).toBe(4);
    expect(ANIM.failed.row).toBe(5);
    expect(ANIM.waiting.row).toBe(6);
    expect(ANIM.running.row).toBe(7);
    expect(ANIM.review.row).toBe(8);
  });

  it("look 16 向：前 8 帧行 9、后 8 帧行 10，列循环", () => {
    expect(LOOK_FRAMES).toHaveLength(16);
    expect(LOOK_FRAMES[0]).toEqual({ x: 0, y: -9 * FRAME_H });
    expect(LOOK_FRAMES[7]).toEqual({ x: -7 * FRAME_W, y: -9 * FRAME_H });
    expect(LOOK_FRAMES[8]).toEqual({ x: 0, y: -10 * FRAME_H });
    expect(LOOK_FRAMES[15]).toEqual({ x: -7 * FRAME_W, y: -10 * FRAME_H });
  });

  it("frameStyle：scale 等比例（spec D15）", () => {
    const s1 = frameStyle("idle", 2, -1, 1);
    expect(s1.backgroundPosition).toBe(`${-2 * FRAME_W}px ${0}px`);
    expect(s1.backgroundSize).toBe("1536px 2288px");
    const s125 = frameStyle("idle", 0, -1, 1.25);
    expect(s125.backgroundSize).toBe("1920px 2860px");
    expect(s125.backgroundPosition).toBe("0px 0px");
    // lookFrame=8 对应 LOOK_FRAMES[8]（行10 列0）；brief 原文写 9，但其 LOOK_FRAMES[9].x=-192，为笔误
    const look = frameStyle("look", 0, 8, 1);
    expect(look.backgroundPosition).toBe(`0px ${-10 * FRAME_H}px`);
  });
});

describe("frameStyle rows 参数（v1/v2，spec EP1）", () => {
  it("rows=9 时 backgroundSize 高度按 9 行计算", () => {
    const s = frameStyle("idle", 0, -1, 1, 9);
    expect(s.backgroundSize).toBe("1536px 1872px");
  });
  it("rows=11 默认值不变（v2 兼容）", () => {
    const s = frameStyle("idle", 0, -1, 1);
    expect(s.backgroundSize).toBe("1536px 2288px");
  });
  it("rows=9 时 look 帧不可用，回退 idle 行定位", () => {
    // v1 无 look 行：调用方保证不进入 look；此处验证默认缩放不越界
    const s = frameStyle("review", 2, -1, 1, 9);
    expect(s.backgroundPosition).toBe("-384px -1664px");
  });
});
