// 升级状态共享 store（prerelease 渠道）：主窗口内弹窗（home 挂载）与标题栏
// 升级徽标共用同一份检查结果，保证一次启动只查一次远端。
// 注意：每个窗口是独立 WebView，store 不跨窗口同步——About 窗口的手动检查
// 用本窗口自己的 store 实例（弹窗同样在该窗口挂载）。
// 「忽略本版本」持久化走 DB settings KV（主题 SSOT 同通道；跨窗口且不受
// WebView 存储回收影响），键名 updater_skipped_version。
import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import {
  checkForUpdates,
  downloadAndInstall,
  type UpdateCheckResult,
  type UpdateInfo,
  type UpdateProgress,
} from "@/lib/updater";

const SKIPPED_SETTING_KEY = "updater_skipped_version";

interface UpdaterStore {
  result: UpdateCheckResult | null;
  checking: boolean;
  downloading: boolean;
  progress: UpdateProgress | null;
  dialogOpen: boolean;
  /** 已忽略的版本号（升级到新版本后自然失效） */
  skippedVersion: string | null;
  /** skippedVersion 是否已从 DB 载入（决定自动弹窗判定是否可信） */
  skippedLoaded: boolean;
  /** 自动检查门栓：dev StrictMode 双挂载也只查一次远端（手动检查不受此门约束） */
  autoCheckDone: boolean;
  checkUpdate: () => Promise<UpdateCheckResult>;
  installUpdate: () => Promise<{ ok: boolean; message?: string }>;
  openDialog: () => void;
  closeDialog: () => void;
  skipVersion: (version: string) => Promise<void>;
  loadSkippedVersion: () => Promise<void>;
}

export const useUpdaterStore = create<UpdaterStore>((set, get) => ({
  result: null,
  checking: false,
  downloading: false,
  progress: null,
  dialogOpen: false,
  skippedVersion: null,
  skippedLoaded: false,
  autoCheckDone: false,

  checkUpdate: async () => {
    set({ checking: true });
    try {
      const result = await checkForUpdates();
      set({ result });
      return result;
    } finally {
      set({ checking: false });
    }
  },

  installUpdate: async () => {
    const { result, downloading } = get();
    // 重入守卫（评审 M7）：下载中重复点击不重复起安装
    if (downloading || result?.status !== "available") {
      return { ok: false, message: "already downloading or no update" };
    }
    set({ downloading: true, progress: null });
    try {
      // Windows：装完插件直接退出进程；macOS：装完 Rust 侧重启——
      // 正常路径上这里不会等到 resolve，失败路径带错误返回由弹窗 toast（评审 I2）
      return await downloadAndInstall(result.update.latestJsonUrl, (progress) =>
        set({ progress }),
      );
    } finally {
      set({ downloading: false });
    }
  },

  openDialog: () => set({ dialogOpen: true }),
  closeDialog: () => set({ dialogOpen: false }),

  skipVersion: async (version) => {
    set({ skippedVersion: version });
    try {
      await invoke("set_setting", { key: SKIPPED_SETTING_KEY, value: version });
    } catch (error) {
      // 持久化失败不影响本进程语义（下次启动可能再弹一次），如实记录
      console.error("persist skipped version failed:", error);
    }
  },

  loadSkippedVersion: async () => {
    try {
      const value = await invoke<string | null>("get_setting", {
        key: SKIPPED_SETTING_KEY,
      });
      set({ skippedVersion: value ?? null, skippedLoaded: true });
    } catch (error) {
      console.error("load skipped version failed:", error);
      set({ skippedLoaded: true });
    }
  },
}));

/** 派生选择器：当前可用更新（无 / 检查失败 / 已是最新 → null；徽标与弹窗共用） */
export function useAvailableUpdate(): UpdateInfo | null {
  return useUpdaterStore((s) =>
    s.result?.status === "available" ? s.result.update : null,
  );
}
