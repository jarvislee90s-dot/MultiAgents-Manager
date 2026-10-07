// Task 12（计划②）· 图集几何与姿态纯层 `src/lib/usage/sheet.ts`（9 用例）。
// 判据来源：计划 §3 第 30 条（图集必须 `fetch → blob → createImageBitmap`，**不得** `<img>`；加载失败
// **不画立绘也要出图**）、第 31 条（帧几何从真实图集尺寸推导，`FRAME_W`/`FRAME_H` 只作自检基准）、
// 第 32 条（`look` 仅 11 行提供；代表帧 `min(col, 该动作帧数 - 1)`；安全默认 `col 0`）、
// brief 步骤 1（`usage.pose.*` 11 键，键序 = `poseKeysFor(11)`）、步骤 2（尺寸 0 / 非整除 / 负 → `null`；
// `rows === 9` 没有 `look` 行 → 必须回落待机，否则画出来是空白格）。
//
// 纪律：主体是纯函数直测（§3 第 34 条：不放 jsdom 专属 API）；只有用例 5 读一次 i18n 文案文件做
// **键序绊线**（`readFileSync` 是 node API，与 `usage-theme-tokens.test.ts` 同款先例）。
import { readFileSync } from "node:fs";
import path from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SHEET_COLS } from "@/components/pet/petAnimations";
import {
  loadSheet,
  poseCol,
  poseFrameCount,
  poseKeysFor,
  resolvePoseRow,
  sheetGeometry,
  type LoadedSheet,
  type SheetBitmap,
  type SheetDeps,
} from "@/lib/usage/sheet";

/** 读 i18n 文案里的 `usage.pose.*` 键序（用例 5 的绊线：键序必须 = `poseKeysFor(11)`） */
function poseKeysInLocale(file: string): string[] {
  const raw = readFileSync(path.join(process.cwd(), "src/i18n/locales", file), "utf8");
  return Object.keys((JSON.parse(raw) as { usage: { pose: Record<string, string> } }).usage.pose);
}

/** 位图假实现：几何推导只读 `width`/`height`（jsdom 无真 ImageBitmap） */
function fakeBitmap(w: number, h: number): SheetBitmap {
  return { width: w, height: h } as unknown as SheetBitmap;
}

/** 假 deps：记录 fetch → blob → createImageBitmap 的调用序（几何与位图必须**同源一次**） */
function fakeDeps(size: { w: number; h: number }): { deps: SheetDeps; calls: string[] } {
  const calls: string[] = [];
  const blob = new Blob([new Uint8Array([1, 2, 3])], { type: "image/webp" });
  return {
    calls,
    deps: {
      fetch: (url: string) => {
        calls.push(`fetch:${url}`);
        return Promise.resolve({
          blob: () => {
            calls.push("blob");
            return Promise.resolve(blob);
          },
        });
      },
      createImageBitmap: (b: Blob) => {
        calls.push("createImageBitmap");
        expect(b).toBe(blob);
        return Promise.resolve(fakeBitmap(size.w, size.h));
      },
    },
  };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("sheet（计划② Task 12：图集几何 + 姿态行/列 + fetch 加载）", () => {
  it("1. 11 行真图集 1536×2288、8 列 → 192×208，且与常量一致（自检基准）", () => {
    expect(sheetGeometry(1536, 2288, 11)).toEqual({
      frameW: 192,
      frameH: 208,
      matchesConstants: true,
    });
  });

  it("2. 9 行真图集 1536×1872 同样推出 192×208（几何由尺寸推导，不硬编码）", () => {
    expect(sheetGeometry(1536, 1872, 9)).toEqual({
      frameW: 192,
      frameH: 208,
      matchesConstants: true,
    });
  });

  it("3. 尺寸 0 / 负 / 非整除 / rows 非法 → null（调用方跳过立绘，不画立绘也要出图）", () => {
    expect(sheetGeometry(0, 0, 11)).toBeNull();
    expect(sheetGeometry(-1536, 2288, 11)).toBeNull();
    expect(sheetGeometry(1536, 0, 11)).toBeNull();
    // 2288 / 9 不整除（真图集只有 1872 / 2288 两种高度）
    expect(sheetGeometry(1536, 2288, 9)).toBeNull();
    expect(sheetGeometry(1537, 2288, 11)).toBeNull();
    // rows 只认 9 / 11（取自 rowsFromSize 的值域）
    expect(sheetGeometry(1536, 2288, 10 as unknown as 11)).toBeNull();
    // cols 非法（0 / 负数）也不得除出 Infinity
    expect(sheetGeometry(1536, 2288, 11, 0)).toBeNull();
  });

  it("4. 与常量不一致**不拦**：仍按推导值给出几何，只把 matchesConstants 置 false", () => {
    // 1600 / 8 = 200、2200 / 11 = 200：几何合法但既不是 192 也不是 208
    expect(sheetGeometry(1600, 2200, 11)).toEqual({
      frameW: 200,
      frameH: 200,
      matchesConstants: false,
    });
  });

  it("5. poseKeysFor：11 行含 look（11 键，键序 = i18n 的 usage.pose.*）；9 行不含（10 键）", () => {
    const eleven = poseKeysFor(11);
    expect(eleven).toEqual([
      "random",
      "idle",
      "run-right",
      "run-left",
      "waving",
      "jumping",
      "failed",
      "waiting",
      "running",
      "review",
      "look",
    ]);
    // 键序绊线：zh / en 两个文案文件的 usage.pose.* 必须与 poseKeysFor(11) 同父同键同序
    expect(poseKeysInLocale("zh.json")).toEqual(eleven);
    expect(poseKeysInLocale("en.json")).toEqual(eleven);

    const nine = poseKeysFor(9);
    expect(nine).toEqual(eleven.filter((k) => k !== "look"));
    expect(nine).toHaveLength(10);
    expect(nine).not.toContain("look");
  });

  it("6. resolvePoseRow：动作键 → 该行；look 仅 11 行图集走第 9 行；9 行 look 与未知键回落待机", () => {
    expect(resolvePoseRow("waving", 11)).toBe(3);
    expect(resolvePoseRow("review", 11)).toBe(8);
    expect(resolvePoseRow("idle", 9)).toBe(0);
    // look 的 16 向扫视在第 9/10 两行，本层只裁第 9 行（仅 11 行图集有该行）
    expect(resolvePoseRow("look", 11)).toBe(9);
    // 9 行图集没有 look 行：回落第 0 行待机，**不得**画空白格（与 FoxbellPet 运行时门控对齐）
    expect(resolvePoseRow("look", 9)).toBe(0);
    // 未知键（含设置里表示「未指定」的 random）一律回落待机行
    expect(resolvePoseRow("does-not-exist", 11)).toBe(0);
    expect(resolvePoseRow("random", 11)).toBe(0);
  });

  it("7. poseFrameCount / poseCol：帧数取该行动画帧数（不是 SHEET_COLS），代表帧 min(col, 帧数-1)", () => {
    expect(poseFrameCount("idle")).toBe(6);
    expect(poseFrameCount("waving")).toBe(4);
    expect(poseFrameCount("run-right")).toBe(8);
    expect(poseFrameCount("run-right")).toBe(SHEET_COLS);
    // look 只裁第 9 行 → 该行有效列数 = 8（16 向跨两行，本层不跨行）
    expect(poseFrameCount("look")).toBe(SHEET_COLS);

    expect(poseCol("idle", 0)).toBe(0);
    expect(poseCol("waving", 9)).toBe(3);
    expect(poseCol("waving", 3)).toBe(3);
    expect(poseCol("idle", -4)).toBe(0);
  });

  it("8. loadSheet：fetch → blob → createImageBitmap 各一次，几何与位图同源（不二次取图）", async () => {
    const { deps, calls } = fakeDeps({ w: 1536, h: 2288 });
    const loaded: LoadedSheet | null = await loadSheet("/pet/spritesheet.webp", 11, deps);

    expect(calls).toEqual(["fetch:/pet/spritesheet.webp", "blob", "createImageBitmap"]);
    expect(loaded?.geometry).toEqual({ frameW: 192, frameH: 208, matchesConstants: true });
    expect(loaded?.bitmap).toEqual({ width: 1536, height: 2288 });
  });

  it("9. loadSheet 降级：空 url / fetch 失败 / 解码失败 / 几何非法 → null；常量不一致只告警", async () => {
    const ok = fakeDeps({ w: 1536, h: 2288 });
    expect(await loadSheet("", 11, ok.deps)).toBeNull();
    expect(ok.calls).toEqual([]); // 空 url 连一次 fetch 都不该发

    const badFetch = fakeDeps({ w: 1536, h: 2288 });
    badFetch.deps.fetch = () => Promise.reject(new Error("asset:// 不可读"));
    expect(await loadSheet("/pet/spritesheet.webp", 11, badFetch.deps)).toBeNull();

    const badDecode = fakeDeps({ w: 1536, h: 2288 });
    badDecode.deps.createImageBitmap = () => Promise.reject(new Error("decode fail"));
    expect(await loadSheet("/pet/spritesheet.webp", 11, badDecode.deps)).toBeNull();

    // 几何非法（2288 / 9 不整除）→ 同样 null，调用方跳过立绘但照常出图
    const badGeom = fakeDeps({ w: 1536, h: 2288 });
    expect(await loadSheet("/pet/spritesheet.webp", 9, badGeom.deps)).toBeNull();

    // 推导值与常量不一致：只告警，**仍按推导值绘制**（返回几何照样是推导出来的那组）
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const mismatch = fakeDeps({ w: 1600, h: 2200 });
    const loaded = await loadSheet("/pet/spritesheet.webp", 11, mismatch.deps);
    expect(loaded?.geometry).toEqual({ frameW: 200, frameH: 200, matchesConstants: false });
    expect(warn).toHaveBeenCalledTimes(1);
  });
});
