import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", async (importOriginal) => {
  const orig = await importOriginal<typeof import("@tauri-apps/api/core")>();
  return { ...orig };
});
vi.mock("../petRuntime", async (importOriginal) => {
  const orig = await importOriginal<typeof import("../petRuntime")>();
  return {
    ...orig,
    probeAudioDurationMs: vi.fn(async (url: string) => {
      if (url.includes("ok")) return 3000;
      throw new Error("probe fail");
    }),
  };
});

import { useVoiceDurationProbe } from "@/components/pet/manage/useVoiceDurationProbe";

const mk = (file: string, durationMs: number | null) => ({
  group: "general",
  name: file,
  file,
  sizeBytes: 10,
  durationMs,
});

describe("useVoiceDurationProbe（第九轮 Bug2）", () => {
  beforeEach(() => vi.clearAllMocks());

  it("dir 非空：null 行并行探测，成功回填、失败保持 null 且不抛错", async () => {
    const rows = [mk("voice/general/ok.mp3", null), mk("voice/done/bad.mp3", null), mk("voice/error/done.m4a", 1500)];
    const setRows = vi.fn();
    renderHook(() => useVoiceDurationProbe(rows, setRows, "/x/p1"));

    await waitFor(() => {
      // hook 以 updater 形态调用 setRows：取最后一次函数形态调用
      const calls = setRows.mock.calls.map((c) => c[0]);
      const updater = [...calls].reverse().find((c): c is (prev: typeof rows) => typeof rows => typeof c === "function");
      expect(updater).toBeTypeOf("function");
      const next = updater(rows);
      expect(next.find((r) => r.file.includes("ok"))?.durationMs).toBe(3000); // 成功回填
      expect(next.find((r) => r.file.includes("bad"))?.durationMs).toBeNull(); // 失败保持 null
      expect(next.find((r) => r.file.includes("done.m4a"))?.durationMs).toBe(1500); // 已有值不动
    });
  });

  it("dir 为空：不探测、不触发 setRows", async () => {
    const setRows = vi.fn();
    renderHook(() => useVoiceDurationProbe([mk("voice/general/ok.mp3", null)], setRows, null));
    await new Promise((r) => setTimeout(r, 50));
    expect(setRows).not.toHaveBeenCalled();
  });
});