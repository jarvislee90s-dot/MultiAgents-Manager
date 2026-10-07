// 升级传输层（prerelease 渠道，2026-10-07 设计定案）：
// - 检查：Rust 侧 GitHub Releases 发现层（semver 挑最大，含 prerelease）
// - 安装：Rust 侧动态端点 + tauri-plugin-updater（签名校验与平台行为不变）
// - 进度：mam-updater-progress 事件广播（安装中 Windows 由插件退出进程、
//   macOS 由 Rust 重启，invoke 的 resolve 仅在失败/非重启路径有意义）
// 浏览器/Playwright 渲染走 tauri-mock.ts 的同名 case。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** 弹窗正文以 release 页 body 为准；htmlUrl 指向具体 tag 页（非 /releases/latest） */
export interface UpdateInfo {
  version: string;
  prerelease: boolean;
  notes: string;
  htmlUrl: string;
  publishedAt: string | null;
  tag: string;
  /** 该 release 的 updater 清单地址，安装命令直接消费 */
  latestJsonUrl: string;
}

export type UpdateCheckResult =
  | { status: "available"; currentVersion: string; update: UpdateInfo }
  | { status: "up-to-date"; currentVersion: string; latestVersion: string }
  | { status: "error"; message: string };

export interface UpdateProgress {
  event: "Started" | "Progress" | "Finished";
  data?: {
    contentLength?: number;
    downloaded?: number;
  };
}

export async function checkForUpdates(): Promise<UpdateCheckResult> {
  try {
    return await invoke<UpdateCheckResult>("check_for_github_update");
  } catch (error) {
    console.error("Failed to check for updates:", error);
    return { status: "error", message: String(error) };
  }
}

interface ProgressEventPayload {
  event: "Started" | "Progress" | "Finished";
  downloaded: number;
  contentLength: number | null;
}

export async function downloadAndInstall(
  latestJsonUrl: string,
  onProgress?: (progress: UpdateProgress) => void,
): Promise<{ ok: boolean; message?: string }> {
  const unlisten: UnlistenFn = await listen<ProgressEventPayload>(
    "mam-updater-progress",
    (event) => {
      onProgress?.({
        event: event.payload.event,
        data: {
          contentLength: event.payload.contentLength ?? undefined,
          downloaded: event.payload.downloaded,
        },
      });
    },
  );
  try {
    await invoke("install_github_update", { latestJsonUrl });
    return { ok: true };
  } catch (error) {
    // 评审 I2：把可操作错误带回 UI 层（Rust 侧文案如「此构建未启用应用内升级…」）
    console.error("Failed to install update:", error);
    return { ok: false, message: String(error) };
  } finally {
    unlisten();
  }
}
