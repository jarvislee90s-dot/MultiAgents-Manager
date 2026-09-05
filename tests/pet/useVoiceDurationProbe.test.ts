import { useState } from "react";
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// @tauri-apps/api/core 由 tests/setup.ts 全局 mock（convertFileSrc → asset://mock），
// 此处不再覆盖，否则真实 convertFileSrc 在 jsdom 下读取 __TAURI_INTERNALS__ 抛错。
vi.mock("@/components/pet/petRuntime", async (importOriginal) => {
  const orig = await importOriginal<typeof import("@/components/pet/petRuntime")>();
  return {
    ...orig,
    probeAudioDurationMs: vi.fn(async (url: string) => {
      if (url.includes("ok")) return 3000;
      throw new Error("probe fail");
    }),
  };
});

import { useVoiceDurationProbe } from "@/components/pet/manage/useVoiceDurationProbe";
import { probeAudioDurationMs } from "@/components/pet/petRuntime";
import type { VoiceRow } from "@/components/pet/petValidation";

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
    const rows = [
      mk("voice/general/ok.mp3", null),
      mk("voice/done/bad.mp3", null),
      mk("voice/error/done.m4a", 1500),
    ];
    // hook 以 updater 形态异步调用 setRows：捕获 updater 后在 act 外手动执行，断言其转换逻辑（不依赖 mock.calls 时序）
    let captured: ((prev: (typeof rows)[number][]) => (typeof rows)[number][]) | null = null;
    const setRows = vi.fn((updater: (prev: (typeof rows)[number][]) => (typeof rows)[number][]) => {
      captured = updater;
    });
    renderHook(() => useVoiceDurationProbe(rows, setRows, "/x/p1"));

    await waitFor(() => expect(captured).not.toBeNull());
    const next = captured!(rows);
    expect(next.find((r) => r.file.includes("ok"))?.durationMs).toBe(3000); // 成功回填
    expect(next.find((r) => r.file.includes("bad"))?.durationMs).toBeNull(); // 失败保持 null
    expect(next.find((r) => r.file.includes("done.m4a"))?.durationMs).toBe(1500); // 已有值不动
  });

  it("dir 为空：不探测、不触发 setRows", async () => {
    const setRows = vi.fn();
    renderHook(() => useVoiceDurationProbe([mk("voice/general/ok.mp3", null)], setRows, null));
    await new Promise((r) => setTimeout(r, 50));
    expect(setRows).not.toHaveBeenCalled();
  });

  it("失败文件不重探（P1-3）：状态真实回流后探测次数收敛", async () => {
    // renderHook 内自持 useState：探测结果经真实 setState 回流触发 effect 重跑，
    // 与线上行为同构（旧实现中失败行 null→新数组→重跑→永远 pending，探测次数持续增长）
    const { result } = renderHook(() => {
      const [rows, setRows] = useState<VoiceRow[]>([
        mk("voice/general/ok.mp3", null),
        mk("voice/done/bad.mp3", null),
      ]);
      useVoiceDurationProbe(rows, setRows, "/x/p1");
      return rows;
    });
    await waitFor(() =>
      expect(result.current.find((r) => r.file.includes("ok"))?.durationMs).toBe(3000)
    );
    const calls = vi.mocked(probeAudioDurationMs).mock.calls.length;
    expect(calls).toBe(2); // ok + bad 各一次
    await new Promise((r) => setTimeout(r, 80));
    expect(vi.mocked(probeAudioDurationMs).mock.calls.length).toBe(calls);
    expect(result.current.find((r) => r.file.includes("bad"))?.durationMs).toBeNull();
  });
});
