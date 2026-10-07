// tests/settings/tailscaleWizard.test.tsx — §C2 Task 6 + §C3 Task 7：Tailscale 首次
// 配置引导向导（src/components/settings/TailscaleWizard.tsx）。覆盖：九步渲染与三态 /
// 需要人的步骤动作文案 / 卡住显示 blocked_reason（不许只转圈）/ Windows 未实机校验
// 弱提示 / login「去登录」链接（MAM 不代登录）/ run_step 触发与回执后重探 /
// funnel 批准链接展示 / verify §C3 可达性校验（尚未生效 + 重试）/ 动作失败落步骤行
//（不再零反馈）/ 头部看板地址按校验态门控 / tailscale 卡展开区集成（Task 9 收敛：
// 向导挂进 RemoteSection 的 tailscale 卡详情区，独立入口行已撤）/ i18n zh-en
// tsWizard 两级键齐备且两 locale 键集相等。
// mock 模式沿用 tests/settings/remoteSection.test.tsx（vi.hoisted + vi.mock）。
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import path from "node:path";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
// useAppTranslation 内部 listen("@tauri-apps/api/event") 在 jsdom 无 Tauri 内核，须 mock
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));

// tests/setup.ts 未初始化 i18n，显式引入并按默认英文断言（jsdom navigator.language=en）
import i18n from "@/i18n";
import { RemoteSection } from "@/components/settings/RemoteSection";
import { TailscaleWizard } from "@/components/settings/TailscaleWizard";

void i18n;

// ---- mock 载荷工厂（形状契约 = Rust 端 tailscale::wizard_status 注释）----
const step = (id: string, needsHuman = false, humanActionKey = "", humanOptional = false) => ({
  id,
  needsHuman,
  humanOptional,
  humanActionKey,
});
const state = (id: string, done: boolean, blockedReason: string | null = null) => ({
  id,
  done,
  blockedReason,
});

// macOS 九步（已实测）基线：detect 完成、download/shields_up 待做、sys_ext 完成、
// login 待做（带 authUrl）、funnel 待做、verify 未验（§C3 校验态全局默认 unverified）、
// autostart 恒完成
const macProbe = (over: Record<string, unknown> = {}) => ({
  platform: "mac",
  windowsVerified: true,
  // I-3：Windows 验证位按覆盖面拆细——windowsVerifiedFrom = 实测覆盖从哪一步起，
  // windowsUnverifiedSteps = 没被端到端实机跑过的步骤（非 Windows 平台恒空）
  windowsVerifiedFrom: null,
  windowsUnverifiedSteps: [],
  // B2：写路径（funnel --bg / reset / 批准链接抓取）实机验证位——macOS 仍为零实机验证
  writePathVerified: false,
  steps: [
    step("detect"),
    step("download"),
    step("install", true, "settings.remote.tsWizard.actAdmin"),
    step("sys_ext", true, "settings.remote.tsWizard.actSysExt"),
    step("login", true, "settings.remote.tsWizard.actLogin"),
    step("shields_up"),
    step("funnel", true, "settings.remote.tsWizard.actFunnel"),
    step("verify"),
    step("autostart"),
  ],
  states: [
    state("detect", true),
    state("download", false),
    state("install", false),
    state("sys_ext", true),
    state("login", false),
    state("shields_up", false),
    state("funnel", false),
    state("verify", false),
    state("autostart", true),
  ],
  authUrl: "https://login.tailscale.com/a/abc123",
  running: false,
  boardUrl: null,
  reach: { state: "unverified" },
  ...over,
});

const stepRow = (id: string) =>
  document.querySelector(`[data-step="${id}"]`) as HTMLElement;

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === "remote_ts_probe") return macProbe();
    return null;
  });
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("TailscaleWizard 步骤渲染与三态（§C2）", () => {
  it("macOS 渲染九步；done 态映射三态点（detect 已完成 / download 待做 / autostart 恒完成）", async () => {
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    const rows = document.querySelectorAll("[data-step]");
    expect(rows.length).toBe(9);
    expect(stepRow("detect").getAttribute("data-done")).toBe("true");
    expect(stepRow("download").getAttribute("data-done")).toBe("false");
    expect(stepRow("autostart").getAttribute("data-done")).toBe("true");
    // 三态文案随行展示
    expect(within(stepRow("detect")).getByText("Done")).toBeTruthy();
    expect(within(stepRow("download")).getByText("To do")).toBeTruthy();
  });

  it("需要人的步骤突出动作文案（后端 humanActionKey 下发 → 前端翻译上墙）", async () => {
    render(<TailscaleWizard />);
    expect(
      await screen.findByText(/prompt for the admin password/i)
    ).toBeTruthy();
    expect(screen.getByText(/approve the Tailscale system extension/i)).toBeTruthy();
    expect(screen.getByText(/complete Tailscale login in your browser/i)).toBeTruthy();
    expect(screen.getByText(/approve it once in your browser/i)).toBeTruthy();
  });

  it("卡住必须点名原因：blockedReason 随行显示「Stuck at this step: 原因」，不许只转圈", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_ts_probe")
        return macProbe({
          states: [
            state("detect", true),
            state("download", false),
            state("install", false),
            state("sys_ext", true),
            state("login", false),
            state("shields_up", false),
            state("funnel", false, "Funnel 已被其他 serve 配置占用（非 MAM）"),
            state("verify", false),
            state("autostart", true),
          ],
        });
      return null;
    });
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    const blocked = within(stepRow("funnel")).getByTestId("ts-blocked");
    expect(blocked.textContent).toContain("Stuck at this step:");
    expect(blocked.textContent).toContain("Funnel 已被其他 serve 配置占用");
    // M4 对照档：无 blockedTone（或 rose）的卡点是**故障档**——红色 + 「卡住」前缀
    expect(blocked.getAttribute("data-blocked-tone")).toBe("rose");
    expect(blocked.className).toMatch(/rose/);
  });

  // M4（2026-10-07 评审 Minor）：卡点的**语义档位**随后端判据走。恢复窗口内的
  // 「后端正在重连…恢复窗口内不判定/不写」是**中间态**（线稿 82-84：既不是故障 rose
  // 也不是正常完成 green），旧实现一律 text-rose-500 + 「卡在这一步：」把每次开机的
  // 正常恢复涂成红色故障。
  it("M4：恢复窗口的卡点走琥珀档（不涂红、不加「卡住」前缀），故障卡点仍走红档", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_ts_probe")
        return macProbe({
          states: [
            state("detect", true),
            // 对照：真故障卡点（无 blockedTone 字段 → fail-safe 到 rose）
            state("download", false, "sha256 校验失败（模拟）"),
            state("install", false),
            state("sys_ext", true),
            state("login", false),
            state("shields_up", false),
            {
              id: "funnel",
              done: false,
              blockedReason:
                "Tailscale 后端正在重连（NoState）——恢复窗口内不判定 Funnel 配置，稍候自动复评",
              blockedTone: "amber",
            },
            state("verify", false),
            state("autostart", true),
          ],
        });
      return null;
    });
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    const amber = within(stepRow("funnel")).getByTestId("ts-blocked");
    expect(amber.getAttribute("data-blocked-tone")).toBe("amber");
    expect(amber.className).toMatch(/amber/);
    expect(amber.className).not.toMatch(/rose/);
    expect(amber.textContent).not.toContain("Stuck at this step:");
    // 成因**原样透出**（后端下发的字符串，前端不翻译不猜）
    expect(amber.textContent).toMatch(/后端正在重连/);
    // 故障卡点不受影响（档位不是一律加琥珀，否则档位本身失去意义）
    const rose = within(stepRow("download")).getByTestId("ts-blocked");
    expect(rose.getAttribute("data-blocked-tone")).toBe("rose");
    expect(rose.className).toMatch(/rose/);
    expect(rose.textContent).toContain("Stuck at this step:");
  });
});

describe("TailscaleWizard Windows 验证位按覆盖面表述（I-3：验证位不得大于证据）", () => {
  // 2026-10-07 真机探测**从第 6 步（shields_up）开始**（安装与登录此前已完成）⇒
  // detect / download / install(UAC) / login 四步的**流程**没被端到端跑过。
  // 旧实现用一个整行布尔把整条提示撤下，等于宣称整条 Windows 流程都验过了。
  it("platform=windows：弱提示照实上墙，并点名列尚未端到端跑过的四步", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_ts_probe")
        return macProbe({
          platform: "windows",
          windowsVerified: false,
          windowsVerifiedFrom: "shields_up",
          windowsUnverifiedSteps: ["detect", "download", "install", "login"],
        });
      return null;
    });
    render(<TailscaleWizard />);
    const hint = await screen.findByTestId("ts-windows-unverified");
    // 文案收窄到精确范围：只说这四步没端到端跑过，不再整行撤下
    expect(hint.textContent).toMatch(/not been run end-to-end/i);
    expect(hint.textContent).toContain("Detect Tailscale");
    expect(hint.textContent).toContain("Install Tailscale");
    expect(hint.textContent).toContain("Sign in to Tailscale");
    // 未被点名的步骤（已实测的后半段）不得出现在"未跑过"清单里
    expect(hint.textContent).not.toContain("Enable Funnel (fixed URL)");
    expect(hint.textContent).not.toContain("Board reachability check");
  });

  it("macOS（本机平台）不显示 Windows 弱提示", async () => {
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    expect(screen.queryByTestId("ts-windows-unverified")).toBeNull();
  });

  it("Windows 整条流程都验过（windowsVerified=true ∧ 未验清单为空）→ 提示撤下", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_ts_probe")
        return macProbe({
          platform: "windows",
          windowsVerified: true,
          windowsVerifiedFrom: null,
          windowsUnverifiedSteps: [],
        });
      return null;
    });
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    expect(screen.queryByTestId("ts-windows-unverified")).toBeNull();
  });
});

describe("TailscaleWizard A1：Funnel 批准是「可能不出现」的分支，不是必经步骤", () => {
  it("可选人工步（humanOptional）带「可能无需此步」弱提示；必需人工步不带", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_ts_probe")
        return macProbe({
          steps: [
            step("detect"),
            step("download"),
            step("install", true, "settings.remote.tsWizard.actAdmin"),
            step("sys_ext", true, "settings.remote.tsWizard.actSysExt"),
            step("login", true, "settings.remote.tsWizard.actLogin"),
            step("shields_up"),
            step("funnel", true, "settings.remote.tsWizard.actFunnel", true),
            step("verify"),
            step("autostart"),
          ],
        });
      return null;
    });
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    const optional = within(stepRow("funnel")).getByTestId("ts-optional-step");
    // 文案必须说清「本平台/本尾网可能无需此步」——不能写成"等你点批准"
    expect(optional.textContent).toMatch(/may not be needed/i);
    // 必需人工步不得带这个提示（否则用户以为可以不点）
    expect(within(stepRow("install")).queryByTestId("ts-optional-step")).toBeNull();
    expect(within(stepRow("login")).queryByTestId("ts-optional-step")).toBeNull();
  });

  it("funnel 回执无批准链接（Windows 实测形态）→ 无批准提示、无错误行、不阻塞", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: { step?: string }) => {
      if (cmd === "remote_ts_probe") return macProbe();
      if (cmd === "remote_ts_run_step" && args?.step === "funnel")
        return { ok: true, approvalUrl: null };
      return null;
    });
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    fireEvent.click(within(stepRow("funnel")).getByRole("button", { name: /run/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("remote_ts_run_step", { step: "funnel" })
    );
    expect(screen.queryByTestId("ts-approval")).toBeNull();
    expect(within(stepRow("funnel")).queryByTestId("ts-step-error")).toBeNull();
  });
});

describe("TailscaleWizard 写路径未实测弱提示（B2：不得把未验证伪装成已验证）", () => {
  it("writePathVerified=false（macOS 现状）→ 顶部弱提示上墙，且说清是「写路径（开通/撤销）」", async () => {
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    const hint = screen.getByTestId("ts-writepath-unverified");
    expect(hint.textContent).toMatch(/write path/i);
    // M-4：macOS 那份"只读探测已实测"的表述只在 macOS 上成立（Linux/Other 会张冠李戴）
    expect(hint.textContent).toMatch(/read-only probes are verified/i);
  });

  it("M-4：非 macOS 且写路径未验证（Linux/Other）→ 文案不得自称「只读探测已实测」", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_ts_probe")
        return macProbe({ platform: "other", writePathVerified: false });
      return null;
    });
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    const hint = screen.getByTestId("ts-writepath-unverified");
    expect(hint.textContent).toMatch(/not been probed on this platform|no probing/i);
    // 本平台零探测：不得出现 macOS 的实测声明（张冠李戴）
    expect(hint.textContent).not.toMatch(/read-only probes are verified/i);
  });

  it("writePathVerified=true（Windows 已实测三条写路径）→ 不显示弱提示", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_ts_probe") return macProbe({ writePathVerified: true });
      return null;
    });
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    expect(screen.queryByTestId("ts-writepath-unverified")).toBeNull();
  });

  it("Windows 整条流程都验过（windowsVerified=true）→ 不再显示「Windows 未校验」旧提示", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_ts_probe")
        return macProbe({
          platform: "windows",
          windowsVerified: true,
          windowsVerifiedFrom: null,
          windowsUnverifiedSteps: [],
        });
      return null;
    });
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    expect(screen.queryByTestId("ts-windows-unverified")).toBeNull();
  });
});

describe("TailscaleWizard 单步触发（remote_ts_run_step）", () => {
  it("可执行步出「Run」按钮，点击调 remote_ts_run_step 并在回执后重探一次", async () => {
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    const probesBefore = invokeMock.mock.calls.filter(
      (c) => c[0] === "remote_ts_probe"
    ).length;
    fireEvent.click(within(stepRow("shields_up")).getByRole("button", { name: /run/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("remote_ts_run_step", { step: "shields_up" })
    );
    // 回执后重探（GET 时机 = 挂载/动作后，不做时刻轮询）
    await waitFor(() => {
      const probes = invokeMock.mock.calls.filter((c) => c[0] === "remote_ts_probe").length;
      expect(probes).toBe(probesBefore + 1);
    });
  });

  it("detect / autostart 不出执行按钮（只读探针 / MAM 自身行为）；verify 走专属重试钮", async () => {
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    for (const id of ["detect", "autostart"]) {
      expect(within(stepRow(id)).queryByRole("button")).toBeNull();
    }
    // verify 未验时出「Retry」重试按钮（调 remote_ts_run_step("verify")，见下组用例）
    expect(within(stepRow("verify")).getByTestId("ts-verify-retry")).toBeTruthy();
  });

  it("login 步：MAM 不代登录——「去登录」为 authUrl 链接（href 原样透传）", async () => {
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    const link = within(stepRow("login")).getByTestId("ts-auth-link") as HTMLAnchorElement;
    expect(link.getAttribute("href")).toBe("https://login.tailscale.com/a/abc123");
    // 链接不是触发命令的按钮——login 步零 invoke
    expect(
      invokeMock.mock.calls.filter((c) => c[0] === "remote_ts_run_step").length
    ).toBe(0);
  });

  it("funnel 回执带批准链接 → 展示批准提示与链接（MAM 只递不代点）", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: { step?: string }) => {
      if (cmd === "remote_ts_probe") return macProbe();
      if (cmd === "remote_ts_run_step" && args?.step === "funnel")
        return { ok: true, approvalUrl: "https://login.tailscale.com/funnel/abc" };
      return null;
    });
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    fireEvent.click(within(stepRow("funnel")).getByRole("button", { name: /run/i }));
    const approval = await screen.findByTestId("ts-approval");
    const link = approval.querySelector("a") as HTMLAnchorElement;
    expect(link.getAttribute("href")).toBe("https://login.tailscale.com/funnel/abc");
  });
});

describe("TailscaleWizard verify §C3（尚未生效 + 重试，Task 7）", () => {
  it("reach=failed：verify 行显示「固定地址尚未生效」+ reason + 重试按钮；点重试调 remote_ts_run_step(\"verify\")", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_ts_probe")
        return macProbe({
          reach: { state: "failed", reason: "公网解析不到该地址" },
          states: [
            state("detect", true),
            state("download", false),
            state("install", false),
            state("sys_ext", true),
            state("login", false),
            state("shields_up", false),
            state("funnel", false),
            state("verify", false, "公网解析不到该地址"),
            state("autostart", true),
          ],
        });
      return null;
    });
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    const row = stepRow("verify");
    // 不得只显示泛用 blocked 行：必须是「尚未生效」口径 + reason（不让用户干等假地址）
    const notLive = within(row).getByTestId("ts-not-live");
    expect(notLive.textContent).toContain("Fixed address not live yet");
    expect(notLive.textContent).toContain("公网解析不到该地址");
    expect(within(row).queryByText("Done")).toBeNull();
    // 重试按钮触发 verify 步（后端自动含自愈）
    fireEvent.click(within(row).getByTestId("ts-verify-retry"));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("remote_ts_run_step", { step: "verify" })
    );
  });

  it("reach=verified：verify 行 Done 且无重试按钮；头部看板地址上墙", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_ts_probe")
        return macProbe({
          running: true,
          boardUrl: "https://a.ts.net/m",
          reach: { state: "verified" },
          states: [
            state("detect", true),
            state("download", false),
            state("install", false),
            state("sys_ext", true),
            state("login", false),
            state("shields_up", false),
            state("funnel", false),
            state("verify", true),
            state("autostart", true),
          ],
        });
      return null;
    });
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    const row = stepRow("verify");
    expect(row.getAttribute("data-done")).toBe("true");
    expect(within(row).queryByTestId("ts-verify-retry")).toBeNull();
    expect(within(row).getByText("Done")).toBeTruthy();
    // 头部地址只在 Verified 显示
    expect(screen.getByText("https://a.ts.net/m")).toBeTruthy();
  });

  it("reach 未验证：running + boardUrl 在场也不显示头部地址（不把未验证地址当可用展示）", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_ts_probe")
        return macProbe({
          running: true,
          boardUrl: "https://a.ts.net/m",
          reach: { state: "unverified" },
        });
      return null;
    });
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    expect(screen.queryByText("https://a.ts.net/m")).toBeNull();
  });

  it("A2：通道在跑但记录未发布 → 给带预期时长的提示（不让用户干等）", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_ts_probe")
        return macProbe({
          running: true,
          boardUrl: "https://a.ts.net/m",
          reach: { state: "unverified" },
        });
      return null;
    });
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    // 实测：首次开通后公网记录约 5–6 分钟才发布，这期间用户唯一感受是"连不上"——
    // 必须告知这是正常现象与预期时长（旧文案只说「尚未生效」，不给时长）
    const hint = screen.getByTestId("ts-pending");
    expect(hint.textContent).toMatch(/5–6 minutes/i);
    expect(hint.textContent).toMatch(/not published yet/i);
  });

  // W-B（真机实测三档时长）：固定一句 5 分钟会把 reset 后重开（实测 30–49 秒）与
  // 开机恢复（记录不撤销、只需等后端重连 1–2 分钟）都误导。文案按**档位**取，
  // 档位由后端载荷给出（reach.state + republish），前端不猜成因。
  it("W-B：reach=record_pending 且 republish=false → 首次开通档（5–6 分钟）", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_ts_probe")
        return macProbe({
          running: true,
          boardUrl: "https://a.ts.net/m",
          reach: { state: "record_pending", republish: false },
        });
      return null;
    });
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    const hint = screen.getByTestId("ts-pending");
    expect(hint.textContent).toMatch(/5–6 minutes/i);
    // M3：首开档照线稿（357 行）**不带** badge——两档不得一律加徽标
    expect(hint.querySelector("span")).toBeNull();
    expect(screen.queryByTestId("ts-republish")).toBeNull();
  });

  it("W-B：reach=record_pending 且 republish=true → 重新开通档（30 秒～1 分钟），不套用 5 分钟", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_ts_probe")
        return macProbe({
          running: true,
          boardUrl: "https://a.ts.net/m",
          reach: { state: "record_pending", republish: true },
        });
      return null;
    });
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    const hint = screen.getByTestId("ts-republish");
    expect(hint.textContent).toMatch(/30 seconds|1 minute/i);
    expect(hint.textContent).not.toMatch(/5–6 minutes/i);
    // M3（2026-10-07 评审）：线稿状态二把「发布中」画成**带琥珀 badge** 的行（wireframe
    // 355），实现必须同形——否则线稿与实现各说各话
    expect(hint.querySelector("span")?.textContent).toBe("Republishing");
    expect(hint.querySelector("span")?.className).toMatch(/amber/);
    // 两档互斥：重开档不得同时挂首开档那块
    expect(screen.queryByTestId("ts-pending")).toBeNull();
  });

  it("W-A：reach=recovering（开机恢复窗口）→ 「恢复中」语义：不是故障、也不是「域名生效中」", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_ts_probe")
        return macProbe({
          running: false,
          boardUrl: null,
          reach: { state: "recovering" },
        });
      return null;
    });
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    const hint = screen.getByTestId("ts-recovering");
    expect(hint.textContent).toMatch(/reconnecting/i);
    expect(hint.textContent).toMatch(/1[–-]2 minutes/i);
    expect(hint.textContent).toMatch(/nothing to do|no action/i);
    // M3：线稿状态二 354 行同形琥珀 badge（与状态五 tsRecoveringBadge 同一档语义）
    expect(hint.querySelector("span")?.textContent).toBe("Reconnecting");
    expect(hint.querySelector("span")?.className).toMatch(/amber/);
    // 成因不得指向「域名发布」（重启后记录不撤销），也不得报成故障/尚未生效
    expect(screen.queryByTestId("ts-pending")).toBeNull();
    expect(screen.queryByTestId("ts-republish")).toBeNull();
    expect(screen.queryByTestId("ts-not-live")).toBeNull();
    expect(screen.queryByText("https://a.ts.net/m")).toBeNull();
  });

  it("A2：通道没在跑（校验态未验证）时不显示「生效中」提示——成因不谎报", async () => {
    render(<TailscaleWizard />); // macProbe 默认 running=false
    await screen.findByText("Detect Tailscale");
    expect(screen.queryByTestId("ts-pending")).toBeNull();
  });

  it("run_step 失败（Err reject）：错误落到对应步骤行的可见错误行（Task 6 移交指针：不再零反馈）", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: { step?: string }) => {
      if (cmd === "remote_ts_probe") return macProbe();
      if (cmd === "remote_ts_run_step" && args?.step === "funnel")
        throw "检测到非 MAM 的 Tailscale serve/Funnel 配置，为避免覆盖已中止开通";
      return null;
    });
    render(<TailscaleWizard />);
    await screen.findByText("Detect Tailscale");
    fireEvent.click(within(stepRow("funnel")).getByRole("button", { name: /run/i }));
    const err = await within(stepRow("funnel")).findByTestId("ts-step-error");
    expect(err.textContent).toContain("Action failed");
    expect(err.textContent).toContain("检测到非 MAM");
  });
});

describe("tailscale 卡展开区集成（Task 9 收敛：向导挂进卡片详情区，独立入口行已撤）", () => {
  const remoteStatusPayload = () => ({
    enabled: false,
    maxDevices: 10,
    channels: {
      local: { running: false, address: "http://127.0.0.1:9420/m" },
      lan: { enabled: false, running: false, addresses: [] },
      quick: { enabled: false, running: false, address: null, error: null },
      named: { enabled: false, running: false, address: null, error: null },
      tailscale: {
        enabled: false,
        running: false,
        address: null,
        error: null,
        reach: { state: "unverified" },
      },
    },
    pin: "4827",
    host: { name: "matebook16s", platform: "mac", version: "0.5.0", bootId: "boot-x" },
    enabledTools: [],
  });
  // detect 未完成 = 未安装态（卡面出「Set up in one click」入口）；autostart 恒完成
  const notInstalledProbe = () =>
    macProbe({
      states: macProbe().states.map((s: { id: string; done: boolean; blockedReason: string | null }) =>
        s.id === "autostart" ? s : { ...s, done: false }
      ),
    });

  it("未点开 tailscale 卡不发起 remote_ts_probe（不偷跑探测）；点卡片 → 展开区拉取探测载荷 → 未安装态 → 点一键配置展开向导", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_status") return remoteStatusPayload();
      if (cmd === "get_setting") return null;
      if (cmd === "remote_devices") return [];
      if (cmd === "remote_ts_probe") return notInstalledProbe();
      return null;
    });
    render(<RemoteSection />);
    expect(
      invokeMock.mock.calls.filter((c) => c[0] === "remote_ts_probe").length
    ).toBe(0);
    fireEvent.click(document.querySelector('[data-card="tailscale"]')!);
    await waitFor(() =>
      expect(invokeMock.mock.calls.some((c) => c[0] === "remote_ts_probe")).toBe(true)
    );
    const phase = await screen.findByTestId("ts-phase");
    // data-phase 由展开区的探测回执**派生**（ts-phase 元素先挂、属性后更）：等它落定再读，
    // 否则读到的是回执到达前的 "unknown"。2026-10-07 定位：把 mock 回执人为延后即 100%
    // 复现（同一类"读到回执到达之前"）；等条件后 100% 绿。断言强度不变（仍要求
    // notInstalled——真回归 / 相位判错照样红）。
    await waitFor(() => expect(phase.getAttribute("data-phase")).toBe("notInstalled"));
    // 向导本体先不挂；一键配置按钮 → 向导上墙
    expect(screen.queryByTestId("ts-wizard")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /set up in one click/i }));
    expect(await screen.findByTestId("ts-wizard")).toBeTruthy();
  });

  it("全步骤完成且校验通过 → 已配置态直接呈现固定地址（无入口行、无向导本体）", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_status")
        return {
          ...remoteStatusPayload(),
          channels: {
            ...remoteStatusPayload().channels,
            tailscale: {
              enabled: true,
              running: true,
              address: "https://mam.tail1234.ts.net/m",
              error: null,
              reach: { state: "verified" },
            },
          },
        };
      if (cmd === "get_setting") return null;
      if (cmd === "remote_devices") return [];
      if (cmd === "remote_ts_probe")
        return macProbe({
          states: macProbe().states.map((s: { id: string }) => ({ ...s, done: true })),
          running: true,
          boardUrl: "https://mam.tail1234.ts.net/m",
          reach: { state: "verified" },
        });
      return null;
    });
    render(<RemoteSection />);
    fireEvent.click(document.querySelector('[data-card="tailscale"]')!);
    const phase = await screen.findByTestId("ts-phase");
    await waitFor(() => expect(phase.getAttribute("data-phase")).toBe("configured"));
    expect(screen.getByText("https://mam.tail1234.ts.net/m")).toBeTruthy();
    expect(screen.queryByTestId("ts-wizard")).toBeNull();
  });
});

// i18n 契约（两级感知版）：TailscaleWizard.tsx 源码引用的每个 settings.remote.* 键
//（含 tsWizard 两级子键）必须在 zh 与 en 同齐备；两 locale 的 settings.remote 子树
// 键集必须相等。RemoteSection 的旧扫描只认单层键——本组件的字面量由本测试把关。
describe("TailscaleWizard i18n zh/en（tsWizard 两级键）", () => {
  // vitest jsdom 环境下 import.meta.url 非 file 协议，用进程 cwd（vitest 以仓库根启动）
  const root = process.cwd();
  const flat = (obj: Record<string, unknown>, prefix = ""): string[] =>
    Object.entries(obj).flatMap(([k, v]) =>
      typeof v === "object" && v !== null
        ? flat(v as Record<string, unknown>, `${prefix}${k}.`)
        : [`${prefix}${k}`]
    );

  it("组件引用键双语齐备 + 两 locale 子树键集相等 + 后端下发的四个动作键在场", () => {
    const zh = JSON.parse(readFileSync(path.join(root, "src/i18n/locales/zh.json"), "utf8"));
    const en = JSON.parse(readFileSync(path.join(root, "src/i18n/locales/en.json"), "utf8"));
    const zhKeys = new Set(flat(zh.settings.remote).map((k) => `settings.remote.${k}`));
    const enKeys = new Set(flat(en.settings.remote).map((k) => `settings.remote.${k}`));
    // 两 locale 键集相等（settings.remote 全子树，含 tsWizard）
    expect([...zhKeys].filter((k) => !enKeys.has(k))).toEqual([]);
    expect([...enKeys].filter((k) => !zhKeys.has(k))).toEqual([]);

    // 两级感知扫描组件源码的字面量键
    const text = readFileSync(
      path.join(root, "src/components/settings/TailscaleWizard.tsx"),
      "utf8"
    );
    const used = new Set<string>();
    for (const m of text.matchAll(/settings\.remote\.[A-Za-z0-9_.]+[A-Za-z0-9_]/g)) {
      used.add(m[0]);
    }
    expect(used.size).toBeGreaterThan(20);
    expect([...used].filter((k) => !zhKeys.has(k))).toEqual([]);
    expect([...used].filter((k) => !enKeys.has(k))).toEqual([]);

    // Rust 端 wizard_steps 的 human_action_key 四值（随载荷下发，前端 t() 动态翻译）
    for (const k of [
      "settings.remote.tsWizard.actAdmin",
      "settings.remote.tsWizard.actSysExt",
      "settings.remote.tsWizard.actLogin",
      "settings.remote.tsWizard.actFunnel",
    ]) {
      expect(zhKeys.has(k)).toBe(true);
      expect(enKeys.has(k)).toBe(true);
    }
  });
});
