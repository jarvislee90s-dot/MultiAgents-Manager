import { useState } from "react";
import { act, renderHook, waitFor } from "@testing-library/react";
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

  describe("失败延迟自动重试（#2：首次瞬时失败不再永久卡徽标）", () => {
    // 注意：fake timers 下 RTL waitFor 的轮询计时器不会自行推进（会挂起），
    // 因此本组全部用 act + advanceTimersByTimeAsync 推进，断言用同步 expect
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("首次探测失败：RETRY_DELAY_MS 内不重探，到点后自动重试，第二次成功则回填时长", async () => {
      const probe = vi.mocked(probeAudioDurationMs);
      probe.mockRejectedValueOnce(new Error("cold start")).mockImplementation(async () => 3000);
      const { result } = renderHook(() => {
        const [rows, setRows] = useState<VoiceRow[]>([mk("voice/general/a.mp3", null)]);
        useVoiceDurationProbe(rows, setRows, "/x/p1");
        return rows;
      });
      expect(probe).toHaveBeenCalledTimes(1); // 首探已发起（effect 同步执行）

      await act(async () => {
        await vi.advanceTimersByTimeAsync(400);
      });
      expect(probe).toHaveBeenCalledTimes(1); // 500ms 重试间隔内不重探

      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });
      expect(probe).toHaveBeenCalledTimes(2); // 到点自动重试
      await act(async () => {
        await Promise.resolve();
      });
      expect(result.current.find((r) => r.file.includes("a.mp3"))?.durationMs).toBe(3000);
    });

    it("重试仍失败：停留 null 且封顶不 reintroduce 死循环（首探 + 2 次自动重试后永久收敛）", async () => {
      // 显式恒失败 mock：clearAllMocks 不清除前序测试留下的 mockImplementation（会污染调用语义）
      vi.mocked(probeAudioDurationMs).mockImplementation(async () => {
        throw new Error("always fail");
      });
      renderHook(() => useVoiceDurationProbe([mk("voice/done/bad.mp3", null)], vi.fn(), "/x/p1"));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });
      expect(probeAudioDurationMs).toHaveBeenCalledTimes(3); // 首探 + 2 次重试，封顶
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
      expect(probeAudioDurationMs).toHaveBeenCalledTimes(3); // 此后不再重探
    });

    it("重试成功（第二次探测即成功）：不再有第三次自动探测", async () => {
      const probe = vi.mocked(probeAudioDurationMs);
      probe.mockRejectedValueOnce(new Error("cold")).mockResolvedValue(3000);
      const { result } = renderHook(() => {
        const [rows, setRows] = useState<VoiceRow[]>([mk("voice/general/a.mp3", null)]);
        useVoiceDurationProbe(rows, setRows, "/x/p1");
        return rows;
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });
      await act(async () => {
        await Promise.resolve();
      });
      expect(probe).toHaveBeenCalledTimes(2);
      expect(result.current.find((r) => r.file.includes("a.mp3"))?.durationMs).toBe(3000);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
      expect(probe).toHaveBeenCalledTimes(2); // 成功后无第三次
    });

    it("挂起窗口内新失败批次并入同一窗口一起重试（Minor 1 回归）", async () => {
      const probe = vi.mocked(probeAudioDurationMs);
      probe.mockReset(); // 清除前序测试残留的 mockImplementation 与 Once 队列（clearAllMocks 不清）
      probe
        .mockRejectedValueOnce(new Error("cold")) // a 首探失败（批次 1，开启窗口）
        .mockRejectedValueOnce(new Error("cold")) // b 首探失败（批次 2，窗口挂起期间加入）
        .mockResolvedValue(3000); // 窗口触发：a、b 一并重试成功
      const setRows = vi.fn();
      const { result, rerender } = renderHook(
        (props: { rows: VoiceRow[] }) => {
          useVoiceDurationProbe(props.rows, setRows, "/x/p1");
          return props.rows;
        },
        { initialProps: { rows: [mk("voice/general/a.mp3", null)] } }
      );
      expect(probe).toHaveBeenCalledTimes(1); // 批次 1（a）

      // 窗口挂起期间新添加的 b 加入 rows（真实添加路径：onAdd 后新行渲染）
      rerender({
        rows: [mk("voice/general/a.mp3", null), mk("voice/general/b.mp3", null)],
      });
      await act(async () => {
        await Promise.resolve();
      });
      expect(probe).toHaveBeenCalledTimes(2); // 批次 2（b）
      expect(result.current[0].durationMs).toBeNull(); // a 仍为 null（窗口未触发）

      // 窗口触发：b 也必须被自动重试（修复前 b 不在挂起定时器捕获列表 → 永久卡徽标）
      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });
      expect(probe).toHaveBeenCalledTimes(4); // 批次 3（a + b 一并重试）
      expect(probe).toHaveBeenNthCalledWith(3, expect.stringContaining("voice/general/a.mp3"));
      expect(probe).toHaveBeenNthCalledWith(4, expect.stringContaining("voice/general/b.mp3"));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
      expect(probe).toHaveBeenCalledTimes(4); // 两批都成功后无后续
    });
  });

  describe("reprobe 手动重测（#2：徽标可点击重测）", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("清除该行记账后立即重新探测，成功回填；失败文件不 reintroduce 循环（新预算封顶）", async () => {
      const probe = vi.mocked(probeAudioDurationMs);
      probe.mockReset(); // 清除前序测试残留的 mockImplementation 与 Once 队列（clearAllMocks 不清）
      probe
        .mockRejectedValueOnce(new Error("cold"))
        .mockRejectedValueOnce(new Error("still"))
        .mockRejectedValueOnce(new Error("again"));
      // 前三次 reject 后的实现显式固定为成功，不依赖前序测试残留的 mockImplementation
      probe.mockResolvedValue(3000);
      const { result } = renderHook(() => {
        const [rows, setRows] = useState<VoiceRow[]>([mk("voice/general/bad.mp3", null)]);
        const reprobe = useVoiceDurationProbe(rows, setRows, "/x/p1");
        return { rows, reprobe };
      });
      // 整轮自动重试耗尽（首探 + 2 次重试均失败）→ 收敛为 null，不再自动探测
      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });
      expect(probe).toHaveBeenCalledTimes(3);
      expect(result.current.rows[0].durationMs).toBeNull();

      // 手动重测：清除该行记账与重试计数 → 立即重新探测（不等 500ms）
      await act(async () => {
        result.current.reprobe("voice/general/bad.mp3");
        await Promise.resolve();
      });
      expect(probe).toHaveBeenCalledTimes(4);
      expect(probe).toHaveBeenNthCalledWith(
        4,
        expect.stringContaining("voice/general/bad.mp3")
      );
      expect(result.current.rows[0].durationMs).toBe(3000); // 手动重测成功回填
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
      expect(probe).toHaveBeenCalledTimes(4); // 成功后再无自动探测
    });
  });

  describe("卸载后不再调度重试定时器（Minor 2 回归）", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("失败探测在卸载后才 resolve：不回填、不调度新定时器（旧 cancelled 语义还原）", async () => {
      const probe = vi.mocked(probeAudioDurationMs);
      probe.mockReset(); // 清除前序测试残留的 mockImplementation 与 Once 队列（clearAllMocks 不清）
      let resolveFirst: ((v: number) => void) | null = null;
      probe.mockImplementationOnce(() => new Promise<number>((res) => (resolveFirst = res)));
      const { unmount } = renderHook(() =>
        useVoiceDurationProbe([mk("voice/general/a.mp3", null)], vi.fn(), "/x/p1")
      );
      expect(probe).toHaveBeenCalledTimes(1); // 首探已发起且挂起

      // 卸载后该探测才 resolve：不得回填、不得调度 500ms 重试定时器
      unmount();
      await act(async () => {
        resolveFirst?.(1);
        await Promise.resolve();
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });
      expect(probe).toHaveBeenCalledTimes(1); // 无重试被调度
    });
  });
});
