// 导出**文件名规则**的唯一用例（计划② Task 11 的 `csvFilename` + 2026-10-06 缺陷修复 X1 的
// `pngFilename`）。独立成文件的原因：命名规则是被 CSV 与分享图**共用**的一条契约
// （「`mam-usage-` 开头」「窗口标识同规则」「唯一性策略」），而 `pngFilename` 属于导出修复、
// `csvFilename` 属于 Task 11 ⇒ 放在 `usage-export-text.test.ts` 里会让两个功能块共用同一个测试文件。
//
// 纯函数、零 IPC、零 DOM。
import { describe, expect, it } from "vitest";
import { csvFilename, pngFilename } from "@/lib/usage/exportText";
import type { UsageRange } from "@/types/usage";

describe("导出文件名（X1：落盘层是裸 fs::write ⇒ 文件名是**防覆盖的唯一一道**）", () => {
  it("csvFilename：恒以 mam-usage- 开头、必以 .csv 结尾（BOM 只认该后缀），且**保持稳定名**", () => {
    expect(csvFilename({ preset: "last7d" })).toBe("mam-usage-last7d.csv");
    expect(csvFilename({ preset: "today" })).toBe("mam-usage-today.csv");
    expect(csvFilename({ preset: "custom", from: "2026-09-01", to: "2026-10-03" })).toBe(
      "mam-usage-2026-09-01_2026-10-03.csv"
    );

    const all: UsageRange[] = [
      { preset: "last5h" },
      { preset: "last30d" },
      { preset: "custom", from: "2026-09-01", to: "2026-10-03" },
      // 防御式：自定义档缺边界时也不得产出不以 .csv 结尾的名字
      { preset: "custom" },
    ];
    for (const r of all) {
      expect(csvFilename(r).startsWith("mam-usage-")).toBe(true);
      expect(csvFilename(r).endsWith(".csv")).toBe(true);
    }
  });

  it("pngFilename：**必带导出时刻**（同名即被静默覆盖），窗口标识与 csvFilename 同规则", () => {
    // 时刻由调用方注入 ⇒ 纯函数可钉（生产侧 `useExportShare` 传默认的 `new Date()`）
    const at = new Date(2026, 9, 6, 23, 1, 42); // 2026-10-06 23:01:42 本地
    expect(pngFilename({ preset: "last7d" }, at)).toBe("mam-usage-last7d-20261006-230142.png");
    expect(pngFilename({ preset: "today" }, at)).toBe("mam-usage-today-20261006-230142.png");
    expect(pngFilename({ preset: "custom", from: "2026-09-01", to: "2026-10-03" }, at)).toBe(
      "mam-usage-2026-09-01_2026-10-03-20261006-230142.png"
    );
    // 自定义档缺边界（防御式）：退化成档位名，仍是合法文件名
    expect(pngFilename({ preset: "custom" }, at)).toBe("mam-usage-custom-20261006-230142.png");

    // ⚠️ **这条是缺陷本身的判据**：以前同一个自定义区间连导两次都是 `mam-usage-custom.png`
    // ⇒ 第二次把第一次**静默覆盖**（界面照样提示「已保存」）。带时刻后两次必然不同名。
    const r: UsageRange = { preset: "custom", from: "2026-09-01", to: "2026-10-03" };
    expect(pngFilename(r, at)).not.toBe(pngFilename(r, new Date(2026, 9, 6, 23, 2, 0)));

    // 与 CSV 的窗口标识**逐字同规则**（只差后缀与时刻）：同一区间两份产物看得出是一对
    const csv = csvFilename({ preset: "custom", from: "2026-09-01", to: "2026-10-03" });
    expect(csv.replace(/\.csv$/, "")).toBe("mam-usage-2026-09-01_2026-10-03");
    expect(pngFilename({ preset: "custom", from: "2026-09-01", to: "2026-10-03" }, at)).toMatch(
      /^mam-usage-2026-09-01_2026-10-03-\d{8}-\d{6}\.png$/
    );
  });
});
