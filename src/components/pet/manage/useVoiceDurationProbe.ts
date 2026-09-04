// useVoiceDurationProbe — 语音时长探测共享 hook（第九轮 Bug2）。
// 导入向导与修改面板共用：对 durationMs===null 的行并行探测并回填（失败保持 null），
// 组件卸载/依赖变化时丢弃过期结果。
import { useEffect } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { probeAudioDurationMs } from "../petRuntime";
import type { VoiceRow } from "../petValidation";

export function useVoiceDurationProbe(
  rows: VoiceRow[],
  setRows: React.Dispatch<React.SetStateAction<VoiceRow[]>>,
  dir: string | null
): void {
  useEffect(() => {
    if (!dir) return;
    const pending = rows.filter((r) => r.durationMs === null);
    if (pending.length === 0) return;
    let cancelled = false;
    void Promise.all(
      pending.map(async (r) => ({
        file: r.file,
        durationMs: await probeAudioDurationMs(convertFileSrc(`${dir}/${r.file}`)).catch(() => null),
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