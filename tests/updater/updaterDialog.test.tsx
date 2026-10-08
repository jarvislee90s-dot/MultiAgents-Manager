// tests/updater/updaterDialog.test.tsx — 升级提醒（prerelease 渠道）
// 覆盖：
// T1 自动弹窗：未忽略 → 启动检查后弹「发现新版本」+ 预发布角标 + markdown 正文渲染（非纯文本）
// T2 自动弹窗抑制：DB 已忽略同版本 → 不自动弹（「忽略本版本」只压弹窗）
// T2b up-to-date 直接抑制：检查结果无更新 → 不弹（评审缺口 3 补直例）
// T3 忽略本版本：写 DB KV（updater_skipped_version）+ 关弹窗
// T4 稍后：仅关弹窗，不写 KV（下次启动再弹）
// T5 立即升级：以检查结果的 latestJsonUrl 调 install_github_update
// T5b 立即升级失败：downloading 复位 + 弹窗保留（评审 I2 回归锁）
// T8 下载中禁关：ESC 不关弹窗（无取消下载手段，误关只留进度黑洞；评审 Minor 回归锁）
// T7 手动二次检查（About 场景）：首查 up-to-date 不弹，再查 available 重开弹窗（评审 I5 回归锁）
// T6 徽标：无更新不渲染；有更新渲染（含被忽略版本）且点击置 dialogOpen
// T9 手动检查失败：toast 带具体原因（Rust message 不吞，弱网/代理/hosts 场景定位用）
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { UpdaterDialog, useManualUpdateCheck } from "@/components/common/updater-dialog";
import { MainTitleBar } from "@/components/common/main-title-bar";
import { useUpdaterStore } from "@/stores/updaterStore";
import type { UpdateCheckResult } from "@/lib/updater";

// toast 断言走 mock（同 bellJump 等既有模式；真 sonner 无 Toaster 挂载时无法断言）
vi.mock("sonner", () => ({
  toast: { error: vi.fn(), info: vi.fn(), success: vi.fn() },
}));

// tests/setup.ts 未初始化 i18n，显式引入并固定中文（文案按 zh 断言）
import i18n from "@/i18n";

// 徽标用例：TitleBar 牵连 webviewWindow API（jsdom 无 __TAURI_INTERNALS__），stub 掉
vi.mock("@/components/common/title-bar", () => ({
  TitleBar: ({ rightActions }: { rightActions?: React.ReactNode }) => (
    <div data-testid="titlebar">{rightActions}</div>
  ),
}));
vi.mock("@/lib/window", () => ({ createWindow: vi.fn(async () => undefined) }));
vi.mock("@/lib/usage/openWindow", () => ({ openUsageDashboard: vi.fn(async () => undefined) }));

const AVAILABLE: Extract<UpdateCheckResult, { status: "available" }> = {
  status: "available",
  currentVersion: "0.4.1",
  update: {
    version: "0.5.0-beta.1",
    prerelease: true,
    notes: "## v0.5.0-beta.1 更新\n\n- **手机远程操控**：发消息与排队\n- **远程审批**：屏读为准\n",
    htmlUrl: "https://github.com/jarvislee90s-dot/MultiAgents-Manager/releases/tag/v0.5.0-beta.1",
    publishedAt: "2026-09-23T15:18:19Z",
    tag: "v0.5.0-beta.1",
    latestJsonUrl:
      "https://github.com/jarvislee90s-dot/MultiAgents-Manager/releases/download/v0.5.0-beta.1/latest.json",
  },
};

/** 每用例的 mock 形态：检查结果 + 已忽略版本（含 install 失败/挂起、check 失败原因） */
const mode = {
  upToDate: false,
  skipped: null as string | null,
  installReject: null as string | null,
  installHang: false,
  checkError: null as string | null,
};

beforeAll(async () => {
  await i18n.changeLanguage("zh");
});

beforeEach(() => {
  // mode 是模块级可变对象：改过的值必须复位，否则泄漏到后续用例（同版本被当成已忽略）
  mode.upToDate = false;
  mode.skipped = null;
  mode.installReject = null;
  mode.installHang = false;
  mode.checkError = null;
  vi.mocked(toast.error).mockClear();
  vi.mocked(toast.info).mockClear();
  vi.mocked(toast.success).mockClear();
  useUpdaterStore.setState({
    result: null,
    checking: false,
    downloading: false,
    progress: null,
    dialogOpen: false,
    skippedVersion: null,
    skippedLoaded: false,
    autoCheckDone: false,
  });
  vi.mocked(invoke).mockImplementation((cmd: string, args?: unknown) => {
    switch (cmd) {
      case "check_for_github_update":
        if (mode.checkError) {
          return Promise.resolve({ status: "error", message: mode.checkError });
        }
        return Promise.resolve(
          mode.upToDate
            ? {
                status: "up-to-date",
                currentVersion: "0.5.0-beta.1",
                latestVersion: "0.5.0-beta.1",
              }
            : AVAILABLE
        );
      case "get_setting":
        return Promise.resolve(
          (args as { key?: string })?.key === "updater_skipped_version" ? mode.skipped : null
        );
      case "set_setting":
        return Promise.resolve(undefined);
      case "install_github_update":
        if (mode.installHang) return new Promise(() => {});
        return mode.installReject ? Promise.reject(mode.installReject) : Promise.resolve(undefined);
      default:
        return Promise.resolve(undefined);
    }
  });
});

/** 等待启动检查链路（get_setting + check）落地 */
async function waitForCheck() {
  await waitFor(() => {
    expect(useUpdaterStore.getState().result?.status).toBe(
      mode.upToDate ? "up-to-date" : "available"
    );
    expect(useUpdaterStore.getState().skippedLoaded).toBe(true);
  });
}

describe("UpdaterDialog 自动模式（home 挂载）", () => {
  it("T1 未忽略时自动弹窗：预发布角标 + markdown 正文渲染 + 三按钮", async () => {
    render(<UpdaterDialog />);

    expect(await screen.findByText("发现新版本")).toBeInTheDocument();
    // 预发布角标
    expect(screen.getByText("预发布")).toBeInTheDocument();
    // 版本行带当前版本对照
    expect(screen.getByText(/新版本 0\.5\.0-beta\.1 可用 · 当前 0\.4\.1/)).toBeInTheDocument();
    // markdown 渲染：加粗生效（不出现字面 **），列表内容在
    expect(screen.getByText("手机远程操控")).toBeInTheDocument();
    expect(screen.queryByText(/\*\*手机远程操控\*\*/)).not.toBeInTheDocument();
    // 三按钮语义
    expect(screen.getByRole("button", { name: "忽略本版本" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "稍后" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "立即升级" })).toBeInTheDocument();
  });

  it("T2 已忽略同版本时不自动弹（忽略只压弹窗，不否定可用更新）", async () => {
    mode.skipped = "0.5.0-beta.1";
    render(<UpdaterDialog />);

    await waitForCheck();
    // 弹窗未开
    expect(screen.queryByText("发现新版本")).not.toBeInTheDocument();
    // 可用更新本身仍在（徽标数据源）
    expect(useUpdaterStore.getState().result?.status).toBe("available");
  });

  it("T2b up-to-date 时直接不弹（无更新 = 无弹窗）", async () => {
    mode.upToDate = true;
    render(<UpdaterDialog />);

    await waitForCheck();
    expect(screen.queryByText("发现新版本")).not.toBeInTheDocument();
    expect(useUpdaterStore.getState().result?.status).toBe("up-to-date");
  });

  it("T3 忽略本版本按钮：写 DB KV 并关弹窗", async () => {
    render(<UpdaterDialog />);
    fireEvent.click(await screen.findByRole("button", { name: "忽略本版本" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("set_setting", {
        key: "updater_skipped_version",
        value: "0.5.0-beta.1",
      });
    });
    expect(useUpdaterStore.getState().skippedVersion).toBe("0.5.0-beta.1");
    await waitFor(() => {
      expect(screen.queryByText("发现新版本")).not.toBeInTheDocument();
    });
  });

  it("T4 稍后按钮：仅关弹窗不写 KV", async () => {
    render(<UpdaterDialog />);
    fireEvent.click(await screen.findByRole("button", { name: "稍后" }));

    await waitFor(() => {
      expect(screen.queryByText("发现新版本")).not.toBeInTheDocument();
    });
    const calls = vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "set_setting");
    expect(calls).toHaveLength(0);
    expect(useUpdaterStore.getState().skippedVersion).toBeNull();
  });

  it("T5 立即升级：以检查结果的 latestJsonUrl 调安装命令", async () => {
    render(<UpdaterDialog />);
    fireEvent.click(await screen.findByRole("button", { name: "立即升级" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("install_github_update", {
        latestJsonUrl:
          "https://github.com/jarvislee90s-dot/MultiAgents-Manager/releases/download/v0.5.0-beta.1/latest.json",
      });
    });
  });

  it("T5b 立即升级失败：downloading 复位、弹窗保留待重试（I2 回归锁）", async () => {
    mode.installReject = "此构建未启用应用内升级";
    render(<UpdaterDialog />);
    fireEvent.click(await screen.findByRole("button", { name: "立即升级" }));

    await waitFor(() => {
      expect(useUpdaterStore.getState().downloading).toBe(false);
    });
    // 弹窗保留：标题与按钮仍在，用户可重试或改去 GitHub
    expect(await screen.findByText("发现新版本")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "立即升级" })).toBeInTheDocument();
  });

  it("T8 下载中 ESC 不关弹窗：进度界面保持可见（评审 Minor 回归锁）", async () => {
    mode.installHang = true;
    render(<UpdaterDialog />);
    fireEvent.click(await screen.findByRole("button", { name: "立即升级" }));

    // 下载挂起中：标题切到下载态，弹窗必须开着
    expect(await screen.findByText("正在下载更新")).toBeInTheDocument();
    await waitFor(() => {
      expect(useUpdaterStore.getState().downloading).toBe(true);
    });

    fireEvent.keyDown(document, { key: "Escape" });
    expect(useUpdaterStore.getState().dialogOpen).toBe(true);
    expect(screen.getByText("正在下载更新")).toBeInTheDocument();
  });

  it("T7 手动模式二次检查：首查 up-to-date 不弹，再查 available 重开（I5 回归锁）", async () => {
    mode.upToDate = true;
    render(<UpdaterDialog manualCheck />);

    // 首查：无更新 → 不弹
    await useUpdaterStore.getState().checkUpdate();
    await waitFor(() => {
      expect(useUpdaterStore.getState().result?.status).toBe("up-to-date");
    });
    expect(screen.queryByText("发现新版本")).not.toBeInTheDocument();

    // 再查（远端出了新版本）：必须重新弹窗，不能被一次性标记吞掉
    mode.upToDate = false;
    await useUpdaterStore.getState().checkUpdate();
    await waitFor(() => {
      expect(useUpdaterStore.getState().result?.status).toBe("available");
    });
    expect(await screen.findByText("发现新版本")).toBeInTheDocument();
  });

  it("T9 手动检查失败：toast 带具体原因，不吞 Rust 侧错误细节", async () => {
    mode.checkError = "GitHub API 请求失败: connection refused";
    // About 页的按钮形态：useManualUpdateCheck 消费检查结果并 toast
    function ManualProbe() {
      const { checkUpdate, checking } = useManualUpdateCheck();
      return (
        <button onClick={checkUpdate} disabled={checking}>
          检查更新
        </button>
      );
    }
    render(<ManualProbe />);
    fireEvent.click(screen.getByRole("button", { name: "检查更新" }));

    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith(
        "检查更新失败：GitHub API 请求失败: connection refused"
      );
    });
    // 失败不弹升级弹窗（store 只落 error 结果）
    expect(useUpdaterStore.getState().result?.status).toBe("error");
    expect(screen.queryByText("发现新版本")).not.toBeInTheDocument();
  });
});

describe("标题栏升级徽标", () => {
  it("T6 无更新不渲染；有更新渲染（含被忽略版本）且点击打开弹窗", async () => {
    // 无更新：不渲染徽标（直接置检查结果，徽标不依赖自动检查）
    useUpdaterStore.setState({
      result: {
        status: "up-to-date",
        currentVersion: "0.5.0-beta.1",
        latestVersion: "0.5.0-beta.1",
      },
    });
    const { unmount } = render(<MainTitleBar />);
    expect(screen.queryByTitle(/发现新版本/)).not.toBeInTheDocument();
    unmount();

    // 有更新（且该版本已被忽略——徽标不受忽略影响）：渲染 + 点击置 dialogOpen
    useUpdaterStore.setState({
      result: AVAILABLE,
      skippedVersion: "0.5.0-beta.1",
      skippedLoaded: true,
    });
    render(<MainTitleBar />);

    const badge = screen.getByTitle("发现新版本 0.5.0-beta.1，点击查看");
    fireEvent.click(badge);
    expect(useUpdaterStore.getState().dialogOpen).toBe(true);
  });
});
