// useVoiceDurationProbe — 语音时长探测共享 hook（第九轮 Bug2）。
// 导入向导与修改面板共用：对 durationMs===null 的行并行探测并回填（失败保持 null，
// UI 按 no-duration 徽标呈现）；失败文件记入 attempted 集合不再重探（P1-3：
// 失败回填生成新数组会再次触发本 effect，无去重则对损坏文件形成无限探测循环）。
import { useEffect, useRef } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { probeAudioDurationMs } from "../petRuntime";
import type { VoiceRow } from "../petValidation";

export function useVoiceDurationProbe(
  rows: VoiceRow[],
  setRows: React.Dispatch<React.SetStateAction<VoiceRow[]>>,
  dir: string | null
): void {
  const attemptedRef = useRef<{ dir: string | null; files: Set<string> }>({
    dir: null,
    files: new Set(),
  });

  useEffect(() => {
    if (!dir) return;
    if (attemptedRef.current.dir !== dir) attemptedRef.current = { dir, files: new Set() };
    const pending = rows.filter(
      (r) => r.durationMs === null && !attemptedRef.current.files.has(r.file)
    );
    if (pending.length === 0) return;
    for (const r of pending) attemptedRef.current.files.add(r.file);
    let cancelled = false;
    void Promise.all(
      pending.map(async (r) => ({
        file: r.file,
        durationMs: await probeAudioDurationMs(convertFileSrc(`${dir}/${r.file}`)).catch(
          () => null
        ),
      }))
    ).then((probed) => {
      if (cancelled) return;
      setRows((prev) =>
        prev.map((r) => {
          const hit = probed.find((p) => p.file === r.file);
          return hit ? { ...r, durationMs: hit.durationMs } : r;
        })
      );
    });
    return () => {
      cancelled = true;
    };
  }, [dir, rows, setRows]);
}
