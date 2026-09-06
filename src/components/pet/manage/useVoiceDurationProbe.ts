// useVoiceDurationProbe — 语音时长探测共享 hook（第九轮 Bug2 + #2 修订）。
// 导入向导与修改面板共用：对 durationMs===null 的行并行探测并回填（失败保持 null，
// UI 按 no-duration 徽标呈现）。失败文件记入 attempted 集合（P1-3：失败回填生成
// 新数组会再次触发本 effect，无去重则对损坏文件形成无限探测循环）。
// #2 修订：首次失败不再永久卡徽标——每行自动延迟重试（500ms 间隔，封顶 2 次），
// 仍失败才停留 null；徽标可点击 reprobe 手动重测（清除该行记账与重试计数重新探测）。
// #2 复审修订：重试定时器触发时全量扫描 failed 记账（而非捕获触发时刻的失败批次）——
// 挂起窗口内新失败批次（新添加/reprobe 的文件）也会被并入同一窗口一并自动重试；
// 探测成功的文件即时移出记账，不再被后续窗口扫到。卸载后（mountedRef）不再调度。
import { useCallback, useEffect, useRef } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { probeAudioDurationMs } from "../petRuntime";
import type { VoiceRow } from "../petValidation";

export const PROBE_RETRY_DELAY_MS = 500;
export const PROBE_MAX_AUTO_RETRIES = 2;

export function useVoiceDurationProbe(
  rows: VoiceRow[],
  setRows: React.Dispatch<React.SetStateAction<VoiceRow[]>>,
  dir: string | null
): (file: string) => void {
  const attemptedRef = useRef<{ dir: string | null; failed: Map<string, number> }>({
    dir: null,
    failed: new Map(),
  });
  const inFlightRef = useRef<Set<string>>(new Set());
  const timerRef = useRef<number | null>(null);
  const mountedRef = useRef(true);
  // 闭包内异步回调读取最新状态，避免竞态读取陈旧闭包值（重试/probe/reprobe 共用）。
  // react-hooks 编译期规则禁止渲染期写 ref，改在 effect 内同步——effect 先于任何
  // 异步回调（探测 promise/计时器）执行，语义不变
  const stateRef = useRef({ rows, setRows, dir });
  useEffect(() => {
    stateRef.current = { rows, setRows, dir };
  });

  const probe = useCallback((files: string[]) => {
    const { rows: latest, dir: currentDir } = stateRef.current;
    if (!currentDir) return;
    const pending = files.filter(
      (f) =>
        !inFlightRef.current.has(f) && latest.some((r) => r.file === f && r.durationMs === null)
    );
    if (pending.length === 0) return;
    for (const f of pending) inFlightRef.current.add(f);
    void Promise.all(
      pending.map(async (r) => ({
        file: r,
        durationMs: await probeAudioDurationMs(convertFileSrc(`${currentDir}/${r}`)).catch(
          () => null
        ),
      }))
    ).then((probed) => {
      if (!mountedRef.current) return; // 已卸载：不再回填/调度（旧 cancelled 语义还原，Minor 2）
      const failedNow: string[] = [];
      for (const p of probed) {
        inFlightRef.current.delete(p.file);
        if (p.durationMs === null) failedNow.push(p.file);
      }
      const state = attemptedRef.current;
      if (state.dir !== stateRef.current.dir) return; // 换目录：旧结果不作数
      stateRef.current.setRows((prev) =>
        prev.map((r) => {
          const hit = probed.find((p) => p.file === r.file);
          return hit ? { ...r, durationMs: hit.durationMs } : r;
        })
      );
      // 探测成功即移出记账（该行 durationMs 已回填，不会被后续窗口再探）
      for (const p of probed) {
        if (p.durationMs !== null) state.failed.delete(p.file);
      }
      if (failedNow.length === 0) return; // 全部成功：无待重试
      for (const f of failedNow) {
        state.failed.set(f, (state.failed.get(f) ?? 0) + 1);
      }
      // 无论是否有挂起定时器，记账都已包含本批失败；窗口触发时全量扫描，
      // 因此挂起窗口内后失败的新批次（新添加/reprobe）也会被并入一并重试（Minor 1）
      if (timerRef.current === null) {
        timerRef.current = window.setTimeout(() => {
          timerRef.current = null;
          const retry = [...state.failed.entries()]
            .filter(([, count]) => count <= PROBE_MAX_AUTO_RETRIES)
            .map(([file]) => file);
          if (retry.length > 0) probe(retry);
        }, PROBE_RETRY_DELAY_MS);
      }
    });
  }, []); // 仅经 ref 访问最新状态，probe 恒稳定（useCallback）

  useEffect(() => {
    if (!dir) return;
    const state = attemptedRef.current;
    if (state.dir !== dir) {
      state.dir = dir;
      state.failed.clear();
      inFlightRef.current.clear();
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    }
    const pending = rows.filter((r) => r.durationMs === null && !state.failed.has(r.file));
    if (pending.length > 0) {
      for (const r of pending) state.failed.set(r.file, 0);
      probe(pending.map((r) => r.file));
    }
  }, [dir, rows, probe]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false; // Minor 2：卸载后 .then 短路，不再回填/调度定时器
      if (timerRef.current !== null) window.clearTimeout(timerRef.current);
    };
  }, []);

  /** 手动重测：清除该行的 attempted 记账与重试计数，立即重新探测（不等重试间隔） */
  const reprobe = (file: string): void => {
    attemptedRef.current.failed.delete(file);
    probe([file]);
  };

  return reprobe;
}
