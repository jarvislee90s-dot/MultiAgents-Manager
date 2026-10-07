// Task 10（计划②）· 导出落盘唯一接缝 `src/lib/usage/saveFile.ts` 的行为锁（11 用例）。
// 判据来源：契约 §3 的两条导出命令行 + 计划② Task 10「关键坑」五条。
//
//  * **导出必须走 Rust 落盘**：接缝只包装既有 `exportSaveText` / `exportSaveBytes`，
//    **不**走 `<a download>` / Blob 下载 / 对象 URL（wry 未注册 download handler →
//    要么 Cancel 且失败静默，要么窗口被导航到 blob URL 界面跑飞）。
//  * **剪贴板只作副本**：`copyText` / `copyImage` 都不得成为导出的唯一出口 ⇒ 两条副本链路
//    在本文件里被断言「一次 IPC 都不发」（没有落盘主出口时，副本入口不得冒充出口）。
//  * **导出失败是自由文本**：两条导出命令 reject 的是 `String`，不是用量命令的 `{ code, detail }`
//    ⇒ 接缝一律 `String(e)` 透传原文，**不得**翻成 `usage.rpc.*`（错误形状不兼容，翻必失真）。
//  * **断言全部走注入的假实现**（`SaveDeps`）：真剪贴板（`ClipboardItem` 全仓零先例、
//    secure-context 前提未验证）与真落盘都不碰；只有「缺省依赖接线」三处经 `tests/setup.ts`
//    的共享 invoke mock 断言命令名与入参键名（那不是真 IPC，也不会写盘）。
import path from "node:path";
import { describe, expect, it } from "vitest";
import { tauriInvokeMock } from "../msw/tauriMocks";
import {
  base64Of,
  copyImage,
  copyText,
  defaultSaveDeps,
  openContainingDir,
  saveBytes,
  saveText,
  type SaveDeps,
} from "@/lib/usage/saveFile";

const EXPORTS = "/Users/jarvis/Downloads";
const CSV = "groupKey,label\nclaude,1\n";
const CSV_NAME = "mam-usage-last7d.csv";
const PNG_NAME = "mam-usage-today.png";
/** PNG 魔数（真分享图的头 8 字节）；base64 真值经 Buffer 独立算得 */
const PNG_MAGIC = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const PNG_MAGIC_B64 = "iVBORw0KGgo=";
const CSV_FILE = `${EXPORTS}/${CSV_NAME}`;

/** 注入用的全假依赖：默认全成功；每条用例只换掉自己关心的那一格（不碰真 IPC / 真剪贴板 / 真文件）。 */
function fakeDeps(overrides: Partial<SaveDeps> = {}): SaveDeps {
  return {
    exportSaveText: async (name) => `${EXPORTS}/${name}`,
    exportSaveBytes: async (name) => `${EXPORTS}/${name}`,
    revealDir: async () => {},
    clipboardWriteText: async () => {},
    clipboardWriteImage: async () => {},
    ...overrides,
  };
}

describe("导出落盘接缝（src/lib/usage/saveFile.ts）", () => {
  it("1. base64Of：空数组 → 空串（不得返回 undefined / 不得依赖 btoa 的假值）", () => {
    expect(base64Of(new Uint8Array(0))).toBe("");
  });

  it("2. base64Of：小输入逐值对得上（PNG 魔数 / ASCII）", () => {
    expect(base64Of(PNG_MAGIC)).toBe(PNG_MAGIC_B64);
    expect(base64Of(new TextEncoder().encode("hello"))).toBe("aGVsbG8=");
  });

  it("3. base64Of：0.9 MB 分块转换不爆栈，且与手推真值逐字符相等", () => {
    // 分享图典型 0.2–2 MB；本用例取中段 0.9 MB = 3 × 300000 字节。
    const bytes = new Uint8Array(900_000).fill(0x41);
    // 前提断言：一次性展开必然 RangeError——「必须分块」不是风格偏好，是可复现的栈上限
    expect(() => String.fromCharCode(...bytes)).toThrow(RangeError);
    // 0x41 = 'A' ⇒ 每 3 字节 base64 恒为 "QUFB"，整段恰好整除 ⇒ 无尾填充
    const got = base64Of(bytes);
    expect(got).toBe("QUFB".repeat(300_000));
    expect(got).toHaveLength(1_200_000);
  });

  it("4. saveText：文件名与内容按契约序（name 在前）逐字透传，回传落盘绝对路径", async () => {
    let sent: [string, string] | null = null;
    const deps = fakeDeps({
      exportSaveText: async (name, content) => {
        sent = [name, content];
        return `${EXPORTS}/${name}`;
      },
    });
    const out = await saveText(CSV, CSV_NAME, deps);
    expect(sent).toEqual([CSV_NAME, CSV]);
    expect(out).toEqual({ ok: true, path: CSV_FILE });
    expect(path.isAbsolute((out as { path: string }).path)).toBe(true);

    // 缺省依赖必须真的接到契约命令与逐字入参键上（本环境 invoke 走共享 mock，非真 IPC）
    await expect(defaultSaveDeps().exportSaveText(CSV_NAME, CSV)).resolves.toContain(CSV_NAME);
    expect(tauriInvokeMock).toHaveBeenCalledWith("export_save_text", {
      name: CSV_NAME,
      content: CSV,
    });
  });

  it("5. saveText：失败透传自由文本原文（String(e)），不翻 i18n 键、不上抛", async () => {
    const plain = await saveText(
      CSV,
      CSV_NAME,
      fakeDeps({
        exportSaveText: async () => {
          throw "写入失败: 只读目录"; // Rust 侧 Result<_, String> 的形态之一
        },
      })
    );
    expect(plain).toEqual({ ok: false, reason: "failed", error: "写入失败: 只读目录" });

    const boxed = await saveText(
      CSV,
      CSV_NAME,
      fakeDeps({
        exportSaveText: async () => {
          throw new Error("导出文件名过长（>128 字符）");
        },
      })
    );
    expect(boxed).toEqual({
      ok: false,
      reason: "failed",
      error: "Error: 导出文件名过长（>128 字符）",
    });

    // 导出错误形状与用量命令的 `{ code, detail }` 不同形 ⇒ 不得试图翻成 usage.rpc.* 键
    for (const out of [plain, boxed]) {
      expect(out.ok).toBe(false);
      if (!out.ok) expect(out.error).not.toMatch(/^usage\./);
    }
  });

  it("6. saveBytes：字节经 base64Of 变**字符串**后落盘（不得传 number 数组）", async () => {
    let sent: [string, unknown] | null = null;
    const deps = fakeDeps({
      exportSaveBytes: async (name, base64) => {
        sent = [name, base64];
        return `${EXPORTS}/${name}`;
      },
    });
    const out = await saveBytes(PNG_MAGIC, PNG_NAME, deps);
    expect(sent).toEqual([PNG_NAME, PNG_MAGIC_B64]);
    // JSON number 数组会把载荷放大数倍（契约 §3）⇒ 第二参必须是 base64 字符串
    expect(typeof sent![1]).toBe("string");
    expect(Array.isArray(sent![1])).toBe(false);
    expect(out).toEqual({ ok: true, path: `${EXPORTS}/${PNG_NAME}` });

    await expect(defaultSaveDeps().exportSaveBytes(PNG_NAME, PNG_MAGIC_B64)).resolves.toContain(
      PNG_NAME
    );
    expect(tauriInvokeMock).toHaveBeenCalledWith("export_save_bytes", {
      name: PNG_NAME,
      base64: PNG_MAGIC_B64,
    });
  });

  it("7. saveBytes：失败同样透传自由文本原文，且不得静默成功", async () => {
    const out = await saveBytes(
      PNG_MAGIC,
      PNG_NAME,
      fakeDeps({
        exportSaveBytes: async () => {
          throw "导出文件过大（17000000 字节 > 16777216 上限）";
        },
      })
    );
    expect(out).toEqual({
      ok: false,
      reason: "failed",
      error: "导出文件过大（17000000 字节 > 16777216 上限）",
    });
    expect(out.ok).toBe(false);
  });

  it("8. openContainingDir：把**文件路径本身**交给 revealDir（不是父目录），失败原样上抛", async () => {
    let sent: string | null = null;
    const deps = fakeDeps({
      revealDir: async (p) => {
        sent = p;
      },
    });
    await openContainingDir(CSV_FILE, deps);
    expect(sent).toBe(CSV_FILE);
    // 传父目录 = 只开目录、不选中文件（resource.rs 对文件是「在文件管理器里选中该文件」）
    expect(sent).not.toBe(path.dirname(CSV_FILE));

    // 缺省依赖同样把文件路径逐字交给既有 reveal 命令（本环境 invoke 走共享 mock）
    await defaultSaveDeps().revealDir(CSV_FILE);
    expect(tauriInvokeMock).toHaveBeenCalledWith("reveal_dir", { path: CSV_FILE });

    // ① 的口径：定位失败不影响「导出成功」⇒ 本层原样上抛，由调用方决定提示
    await expect(
      openContainingDir(
        CSV_FILE,
        fakeDeps({
          revealDir: async () => {
            throw new Error("路径不在 reveal 白名单内");
          },
        })
      )
    ).rejects.toThrow("路径不在 reveal 白名单内");
  });

  it("9. copyText：只碰剪贴板（零 IPC），成功给 via=clipboard、失败如实报原因", async () => {
    const summary = "用量看板 · 近 7 天\n请求输入(全文累计): 1,954,268\n";
    let sent: string | null = null;
    const out = await copyText(
      summary,
      fakeDeps({
        clipboardWriteText: async (text) => {
          sent = text;
        },
      })
    );
    expect(sent).toBe(summary);
    expect(out).toEqual({ ok: true, via: "clipboard" });
    expect(tauriInvokeMock).not.toHaveBeenCalled();

    // best-effort：剪贴板被拒（非安全上下文 / 无权限）→ 如实报自由文本原因，**不改道落盘**
    const denied = await copyText(
      summary,
      fakeDeps({
        clipboardWriteText: async () => {
          throw new Error("NotAllowedError: Write permission denied.");
        },
      })
    );
    expect(denied).toEqual({
      ok: false,
      error: "Error: NotAllowedError: Write permission denied.",
    });
    expect(tauriInvokeMock).not.toHaveBeenCalled();
  });

  it("10. copyImage：同一个 Blob 交给剪贴板写手（副本成功即 via=clipboard，不带落盘路径）", async () => {
    let sent: Blob | null = null;
    const blob = new Blob([PNG_MAGIC], { type: "image/png" });
    const out = await copyImage(
      blob,
      fakeDeps({
        clipboardWriteImage: async (b) => {
          sent = b;
        },
      })
    );
    // 同一个对象：不做 data URL 中转、不落盘、不重新编码
    expect(sent).toBe(blob);
    expect(out).toEqual({ ok: true, via: "clipboard" });
    // 副本成功也不得冒充落盘出口（没有 path 可言）
    expect("path" in out).toBe(false);
    expect(tauriInvokeMock).not.toHaveBeenCalled();
  });

  it("11. copyImage：ClipboardItem 不可用时如实失败，不得退化成落盘、不得吞错", async () => {
    const blob = new Blob([PNG_MAGIC], { type: "image/png" });
    const out = await copyImage(
      blob,
      fakeDeps({
        clipboardWriteImage: async () => {
          throw new ReferenceError("ClipboardItem is not defined"); // 非安全上下文 / 老 WebView
        },
      })
    );
    expect(out).toEqual({ ok: false, error: "ReferenceError: ClipboardItem is not defined" });
    expect("via" in out).toBe(false);
    expect(tauriInvokeMock).not.toHaveBeenCalled();
  });
});
