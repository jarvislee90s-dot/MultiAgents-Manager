// tests/pet/assets.test.ts — 校验 manifest 与素材文件齐全（spec §6.1）
import { readFileSync, existsSync } from "node:fs";
import { resolve } from "node:path";

const ROOT = resolve(__dirname, "../../public/pet");

describe("pet assets", () => {
  const manifest = JSON.parse(readFileSync(resolve(ROOT, "manifest.json"), "utf-8")) as {
    index: number;
    group: string;
    name: string;
    file: string;
  }[];

  it("精灵图存在", () => {
    expect(existsSync(resolve(ROOT, "spritesheet.webp"))).toBe(true);
  });

  it("manifest 含四个组且索引连续", () => {
    const groups = new Set(manifest.map((v) => v.group));
    expect(groups).toEqual(new Set(["general", "approval", "done", "error"]));
    expect(manifest.map((v) => v.index)).toEqual(manifest.map((_, i) => i));
  });

  it("每个语音文件真实存在且文件名去扩展即 name", () => {
    for (const v of manifest) {
      const p = resolve(ROOT, "voice", v.file);
      expect(existsSync(p)).toBe(true);
      expect(v.name).toBe(v.file.replace(/\.(m4a|mp4)$/i, "").split("/").pop());
    }
  });
});
