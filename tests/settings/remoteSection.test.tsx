// tests/settings/remoteSection.test.tsx — M5 A6：设置页「远程接入」按线稿 v5 重写
//（docs/superpowers/wireframes/2026-09-17-remote-settings-redesign.html 为唯一 UI 契约）。
// 覆盖：四卡渲染与 live 态映射（enabled||running；本机=总开关）/ 点卡片唯一展开 /
// 本机开关锁死 / lan 开关 P7 Dialog 流（Err 特征文案 → 确认 → 重试）/ quick 换址警告 /
// 教程 popover 开收 / PIN 随机与保存与重置设备确认 / 设备行 via 四值徽标与重命名与踢下线 /
// N / 10 上限徽标 / 本机名称默认系统名 / i18n zh-en 无缺键。
// mock 模式沿用本文件旧版（vi.hoisted + vi.mock，自带 invoke/event/sonner/qrcode）。
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import path from "node:path";

const { invokeMock, toastSuccessMock, toastErrorMock, toDataUrlMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  toastSuccessMock: vi.fn(),
  toastErrorMock: vi.fn(),
  toDataUrlMock: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
// useAppTranslation 内部 listen("@tauri-apps/api/event") 在 jsdom 无 Tauri 内核，须 mock
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));
// sonner mock 须放文件顶部（vi.mock 提升语义，见旧版注释）
vi.mock("sonner", () => ({
  toast: Object.assign(vi.fn(), {
    info: vi.fn(),
    success: toastSuccessMock,
    error: toastErrorMock,
  }),
}));
// jsdom 无 canvas 内核，qrcode.toDataURL 出不了真图——mock 成固定 dataURL
vi.mock("qrcode", () => ({
  default: { toDataURL: (...args: unknown[]) => toDataUrlMock(...args) },
}));

// tests/setup.ts 未初始化 i18n，显式引入并按默认英文断言（jsdom navigator.language=en）
import i18n from "@/i18n";
import { RemoteSection } from "@/components/settings/RemoteSection";

void i18n;

// ---- mock 载荷工厂（形状契约 = Rust channels_payload 注释，M5 A5）----
const channelsOf = (over: {
  lan?: Record<string, unknown>;
  quick?: Record<string, unknown>;
  named?: Record<string, unknown>;
} = {}) => ({
  local: { running: true, address: "http://127.0.0.1:9420/m" },
  lan: {
    enabled: true,
    running: true,
    addresses: ["http://192.168.66.202:9420/m", "http://192.168.42.216:9420/m"],
    ...over.lan,
  },
  quick: {
    enabled: true,
    running: true,
    address: "https://quick-test.trycloudflare.com/m",
    error: null,
    ...over.quick,
  },
  named: { enabled: false, running: false, address: null, error: null, ...over.named },
});

const statusOf = (over: Record<string, unknown> = {}) => ({
  enabled: true,
  maxDevices: 10,
  channels: channelsOf(over.channels as never),
  pin: "4827",
  host: { name: "matebook16s", platform: "windows", version: "0.4.2", bootId: "boot-x" },
  enabledTools: [],
  ...over,
});

// 总开关关着的基线（通道 KV 开关仍在——卡片开关态照常渲染）
const disabledStatus = () => statusOf({ enabled: false });

// 默认 invoke 行为：status + 各设置键回填 + 空设备表
beforeEach(() => {
  invokeMock.mockReset();
  toDataUrlMock.mockResolvedValue("data:image/png;base64,mockQR");
  // jsdom 无剪贴板内核：stub writeText（复制按钮 → toast 成功路径）
  Object.defineProperty(window.navigator, "clipboard", {
    value: { writeText: vi.fn(async () => {}) },
    configurable: true,
  });
  invokeMock.mockImplementation(async (cmd: string, args?: { key?: string }) => {
    if (cmd === "remote_status") return statusOf();
    if (cmd === "get_setting") {
      if (args?.key === "remote.tunnel_token") return "eyJh-saved-token";
      return null; // remote.host_name 未设置 → 默认系统名；remote.keepalive → true
    }
    if (cmd === "remote_devices") return [];
    return null;
  });
});

afterEach(() => {
  vi.clearAllMocks();
});

// 卡片定位（线稿四卡一排；data-card 是卡片可点击容器的稳定钩子）
const card = (key: string) =>
  document.querySelector(`[data-card="${key}"]`) as HTMLElement;
const cardSwitch = (key: string) => within(card(key)).getByRole("switch");
const liveDot = (key: string) => card(key).querySelector("[data-live]") as HTMLElement;
// 唯一展开区（同时只显示一个）
const expandedKey = () =>
  document.querySelector("[data-expand]")?.getAttribute("data-expand");

describe("RemoteSection 四卡渲染与 live 态映射（M5 A6）", () => {
  it("四张卡按线稿顺序渲染，live 点与开关态 = enabled||running（本机=总开关）", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_status")
        return statusOf({
          channels: channelsOf({
            lan: { enabled: false, running: false }, // 关 → 灰点 + 开关 off
            quick: { enabled: true, running: false }, // 开但未运行 → 仍算 on（映射含 enabled）
            named: { enabled: false, running: false }, // 关 → 灰点
          }),
        });
      if (cmd === "get_setting") return null;
      if (cmd === "remote_devices") return [];
      return null;
    });
    render(<RemoteSection />);
    // status 已载（总开关 Switch disabled={busy || !status}，载入后才可点）
    const master = screen.getByRole("switch", { name: /enable remote access/i });
    await waitFor(() => expect(master).toBeEnabled());
    // 四卡齐全（本机 / 局域网 / 临时隧道 / 命名隧道）
    for (const key of ["local", "lan", "quick", "named"]) expect(card(key)).toBeTruthy();
    // live 点映射：本机=总开关(开) → 亮；局域网(关) → 灰；临时隧道(enabled) → 亮；命名隧道(关) → 灰
    expect(liveDot("local")).toHaveClass("bg-emerald-500");
    expect(liveDot("lan")).toHaveClass("bg-gray-300");
    expect(liveDot("quick")).toHaveClass("bg-emerald-500");
    expect(liveDot("named")).toHaveClass("bg-gray-300");
    // 开关态与 live 同口径
    expect(cardSwitch("local")).toBeChecked();
    expect(cardSwitch("lan")).not.toBeChecked();
    expect(cardSwitch("quick")).toBeChecked();
    expect(cardSwitch("named")).not.toBeChecked();
  });

  it("点总开关关 → remote_toggle(false)；线稿关闭语义文案随行展示", async () => {
    render(<RemoteSection />);
    const sw = screen.getByRole("switch", { name: /enable remote access/i });
    await waitFor(() => expect(sw).toBeEnabled());
    fireEvent.click(sw);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("remote_toggle", { enabled: false })
    );
    expect(screen.getByText(/Explicitly turning it off/i)).toBeTruthy();
  });
});

describe("RemoteSection 点卡片唯一展开详情区（M5 A6）", () => {
  it("默认展开本机；点局域网切到局域网；点临时隧道切到临时隧道——同一时刻只有一个", async () => {
    render(<RemoteSection />);
    await screen.findByText("Local only"); // 本机详情默认展开（线稿 p-local 带 sel）
    expect(expandedKey()).toBe("local");
    expect(screen.getByText("http://127.0.0.1:9420/m")).toBeTruthy();
    // 点局域网卡 → 切换：局域网详情上屏、本机详情消失
    fireEvent.click(card("lan"));
    expect(expandedKey()).toBe("lan");
    expect(screen.getByText("http://192.168.66.202:9420/m")).toBeTruthy();
    expect(screen.queryByText("Local only")).toBeNull();
    // 点临时隧道卡 → 再切换：换址警告上屏，局域网地址消失（唯一展开）
    fireEvent.click(card("quick"));
    expect(expandedKey()).toBe("quick");
    expect(screen.getByText(/regenerates the tunnel address/i)).toBeTruthy();
    expect(screen.queryByText("http://192.168.66.202:9420/m")).toBeNull();
  });

  it("本机详情：仅本机徽标 + 回环地址 + 复制链接 + 转发终点说明", async () => {
    render(<RemoteSection />);
    expect(await screen.findByText("Local only")).toBeTruthy();
    expect(screen.getByText("http://127.0.0.1:9420/m")).toBeTruthy();
    expect(screen.getByText(/forwarding target for tunnel traffic/i)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /copy link/i }));
    await waitFor(() =>
      expect(toastSuccessMock).toHaveBeenCalled()
    );
  });

  it("局域网详情：推荐徽标落首条地址 + 二维码（地址#pin=<pin>）+ 换网络说明", async () => {
    render(<RemoteSection />);
    await screen.findByText("Local only");
    fireEvent.click(card("lan"));
    // 首条带推荐徽标，第二条不带（getAllByText 恰 1 次）
    expect(screen.getAllByText("Recommended")).toHaveLength(1);
    expect(screen.getByText("http://192.168.42.216:9420/m")).toBeTruthy();
    // 二维码内容 = 地址#pin=<pin>（PIN 来自 remote_status.pin = 4827）
    await waitFor(() =>
      expect(toDataUrlMock).toHaveBeenCalledWith(
        "http://192.168.66.202:9420/m#pin=4827",
        expect.anything()
      )
    );
    expect(screen.getByText(/The address changes when the network changes/i)).toBeTruthy();
  });
});

describe("RemoteSection 本机卡开关锁死（M5 A6）", () => {
  it("本机开关 disabled + title「随开启远程接入常驻」，点击不发起任何通道命令", async () => {
    render(<RemoteSection />);
    await screen.findByText("Local only");
    const sw = cardSwitch("local");
    expect(sw).toBeDisabled();
    expect(sw).toHaveAttribute("title", "Always on while remote access is enabled");
    fireEvent.click(sw);
    expect(invokeMock).not.toHaveBeenCalledWith(
      "remote_toggle_channel",
      expect.anything()
    );
  });
});

describe("RemoteSection lan 开关 P7 TLS Dialog 流（M5 A6）", () => {
  const lanOffStatus = () =>
    statusOf({ channels: channelsOf({ lan: { enabled: false, running: false } }) });

  it("开 lan 收到 P7 特征 Err → 弹 TLS 确认 Dialog，不 toast 报错", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "remote_status") return lanOffStatus();
      if (cmd === "remote_toggle_channel" && args?.channel === "lan" && args?.on === true) {
        throw "对外绑定需先确认已配置 TLS 反向代理（remote_confirm_public）";
      }
      if (cmd === "get_setting") return null;
      if (cmd === "remote_devices") return [];
      return null;
    });
    render(<RemoteSection />);
    await waitFor(() => expect(cardSwitch("lan")).toBeEnabled());
    fireEvent.click(cardSwitch("lan"));
    // 特征文案命中 → 既有 TLS 确认 Dialog（标题即 P7 门槛文案），错误不落 toast
    expect(await screen.findByText("Confirm external binding")).toBeTruthy();
    expect(toastErrorMock).not.toHaveBeenCalled();
  });

  it("Dialog 确认 → remote_confirm_public 先行、重试 remote_toggle_channel(lan,true) 成功、Dialog 关", async () => {
    let lanTries = 0;
    const calls: string[] = [];
    invokeMock.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      calls.push(cmd);
      if (cmd === "remote_status") return lanOffStatus();
      if (cmd === "remote_toggle_channel" && args?.channel === "lan" && args?.on === true) {
        lanTries += 1;
        if (lanTries === 1) throw "对外绑定需先确认已配置 TLS 反向代理（remote_confirm_public）";
        return; // 重试成功
      }
      if (cmd === "get_setting") return null;
      if (cmd === "remote_devices") return [];
      return null;
    });
    render(<RemoteSection />);
    await waitFor(() => expect(cardSwitch("lan")).toBeEnabled());
    fireEvent.click(cardSwitch("lan"));
    fireEvent.click(await screen.findByRole("button", { name: /I understand, enable/i }));
    await waitFor(() => expect(lanTries).toBe(2));
    // 顺序契约：先置位 ack（后端门据此放行）再重试
    expect(calls.indexOf("remote_confirm_public")).toBeGreaterThan(-1);
    expect(calls.indexOf("remote_confirm_public")).toBeLessThan(calls.lastIndexOf("remote_toggle_channel"));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("Dialog 取消 → 不调 remote_confirm_public、不重试，仅关 Dialog", async () => {
    let lanTries = 0;
    invokeMock.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "remote_status") return lanOffStatus();
      if (cmd === "remote_toggle_channel" && args?.channel === "lan" && args?.on === true) {
        lanTries += 1;
        throw "对外绑定需先确认已配置 TLS 反向代理（remote_confirm_public）";
      }
      if (cmd === "get_setting") return null;
      if (cmd === "remote_devices") return [];
      return null;
    });
    render(<RemoteSection />);
    await waitFor(() => expect(cardSwitch("lan")).toBeEnabled());
    fireEvent.click(cardSwitch("lan"));
    fireEvent.click(await screen.findByRole("button", { name: /^cancel$/i }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(invokeMock).not.toHaveBeenCalledWith("remote_confirm_public");
    // 仅首次触发的那一次调用，无重试
    await act(async () => {});
    expect(lanTries).toBe(1);
  });

  it("非 lan 通道开关失败走 toast 报错，不弹 Dialog（P7 Dialog 仅 lan 开启路径）", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "remote_status") return lanOffStatus();
      if (cmd === "remote_toggle_channel" && args?.channel === "quick") {
        throw "隧道拉起失败（模拟）";
      }
      if (cmd === "get_setting") return null;
      if (cmd === "remote_devices") return [];
      return null;
    });
    render(<RemoteSection />);
    await waitFor(() => expect(cardSwitch("quick")).toBeEnabled());
    // quick 默认开着（enabled）→ 点击为关方向；失败仍走 toast 而非 Dialog
    fireEvent.click(cardSwitch("quick"));
    await waitFor(() => expect(toastErrorMock).toHaveBeenCalled());
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("remote_toggle_channel", { channel: "quick", on: false });
  });
});

describe("RemoteSection quick 换址警告与运行态（M5 A6）", () => {
  it("运行中：公网徽标 + 地址 + 运行中/断线自动重连 + 换址警告块 + 二维码", async () => {
    render(<RemoteSection />);
    await screen.findByText("Local only");
    fireEvent.click(card("quick"));
    expect(screen.getAllByText("Public").length).toBeGreaterThan(0);
    expect(screen.getByText("https://quick-test.trycloudflare.com/m")).toBeTruthy();
    expect(screen.getByText("Running")).toBeTruthy();
    expect(screen.getByText(/auto-reconnects on disconnect/i)).toBeTruthy();
    expect(screen.getByText(/regenerates the tunnel address/i)).toBeTruthy();
    await waitFor(() =>
      expect(toDataUrlMock).toHaveBeenCalledWith(
        "https://quick-test.trycloudflare.com/m#pin=4827",
        expect.anything()
      )
    );
    expect(screen.getByText(/cellular/i)).toBeTruthy();
  });

  it("错误态：error 原文展示、无「运行中」徽标", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_status")
        return statusOf({
          channels: channelsOf({
            quick: { enabled: true, running: false, address: null, error: "cloudflared 下载失败" },
          }),
        });
      if (cmd === "get_setting") return null;
      if (cmd === "remote_devices") return [];
      return null;
    });
    render(<RemoteSection />);
    await screen.findByText("Local only");
    fireEvent.click(card("quick"));
    expect(await screen.findByText("cloudflared 下载失败")).toBeTruthy();
    expect(screen.queryByText("Running")).toBeNull();
  });
});

describe("RemoteSection 命名隧道：Token 保存与教程 popover（M5 A6）", () => {
  it("Token 输入回填已存值，保存走 set_setting(remote.tunnel_token)", async () => {
    render(<RemoteSection />);
    await screen.findByText("Local only");
    fireEvent.click(card("named"));
    const input = (await screen.findByLabelText("Tunnel Token")) as HTMLInputElement;
    await waitFor(() => expect(input.value).toBe("eyJh-saved-token"));
    fireEvent.change(input, { target: { value: "eyJh-new-token" } });
    // Token 保存按钮与 PIN 保存同名（线稿均为「保存」），作用域限定在 Token 输入框所在行
    fireEvent.click(
      within(input.closest("div")!).getByRole("button", { name: /^save$/i })
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("set_setting", {
        key: "remote.tunnel_token",
        value: "eyJh-new-token",
      })
    );
    expect(toastSuccessMock).toHaveBeenCalled();
  });

  it("教程 popover：点击展开六步教程，再点收起（可保持展开边看边操作）", async () => {
    render(<RemoteSection />);
    await screen.findByText("Local only");
    fireEvent.click(card("named"));
    // 收起态：教程正文不在文档
    expect(screen.queryByText(/Prerequisite: a domain hosted on Cloudflare/i)).toBeNull();
    fireEvent.click(screen.getByText(/How-to \(click to expand\/collapse\)/i));
    // 展开：前置 + 六步 + 收尾说明；data-help 钩子随开合翻转（A8 评审：勿留死钩子）
    expect(screen.getByText(/Prerequisite: a domain hosted on Cloudflare/i)).toBeTruthy();
    expect(
      (screen.getByText(/How-to \(click to expand\/collapse\)/i) as HTMLElement).dataset["help"]
    ).toBe("open");
    expect(screen.getByText(/Sign in at dash\.cloudflare\.com/i)).toBeTruthy();
    expect(screen.getByText(/Create a tunnel, connection type Cloudflared/i)).toBeTruthy();
    expect(screen.getByText(/Save tunnel;/i)).toBeTruthy();
    expect(screen.getByText(/install nothing/i)).toBeTruthy();
    expect(screen.getByText(/Public Hostname tab/i)).toBeTruthy();
    expect(screen.getByText(/paste the token into the input and click Save/i)).toBeTruthy();
    expect(screen.getByText(/never changes afterwards/i)).toBeTruthy();
    // 再点收起
    fireEvent.click(screen.getByText(/How-to \(click to expand\/collapse\)/i));
    expect(screen.queryByText(/Prerequisite: a domain hosted on Cloudflare/i)).toBeNull();
  });

  it("命名隧道详情含公网地址与二维码；错误态展示 error 原文", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_status")
        return statusOf({
          channels: channelsOf({
            named: {
              enabled: true,
              running: true,
              address: "https://mam.jarvis.example.com/m",
              error: null,
            },
          }),
        });
      if (cmd === "get_setting") return null;
      if (cmd === "remote_devices") return [];
      return null;
    });
    render(<RemoteSection />);
    await screen.findByText("Local only");
    fireEvent.click(card("named"));
    expect(screen.getByText("https://mam.jarvis.example.com/m")).toBeTruthy();
    await waitFor(() =>
      expect(toDataUrlMock).toHaveBeenCalledWith(
        "https://mam.jarvis.example.com/m#pin=4827",
        expect.anything()
      )
    );
  });
});

describe("RemoteSection 访问密码（M5 A6）", () => {
  const pinInput = () => screen.getByLabelText("Access PIN") as HTMLInputElement;

  it("PIN 输入框回填 remote_status.pin（4 位数字居中）", async () => {
    render(<RemoteSection />);
    await waitFor(() => expect(pinInput().value).toBe("4827"));
  });

  it("随机按钮生成 1000-9999 的 4 位数字", async () => {
    render(<RemoteSection />);
    await waitFor(() => expect(pinInput().value).toBe("4827"));
    for (let i = 0; i < 20; i++) {
      fireEvent.click(screen.getByRole("button", { name: /random/i }));
      const v = pinInput().value;
      expect(v).toMatch(/^\d{4}$/);
      expect(Number(v)).toBeGreaterThanOrEqual(1000);
      expect(Number(v)).toBeLessThanOrEqual(9999);
    }
  });

  it("保存 → remote_set_pin 以输入值调用 + 成功提示「所有设备需重新输入」语义", async () => {
    render(<RemoteSection />);
    await waitFor(() => expect(pinInput().value).toBe("4827"));
    fireEvent.change(pinInput(), { target: { value: "1357" } });
    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("remote_set_pin", { pin: "1357" }));
    await waitFor(() =>
      expect(toastSuccessMock).toHaveBeenCalledWith(
        expect.stringMatching(/all devices must re-enter/i)
      )
    );
  });

  it("保存未满 4 位时禁用（A8 评审：不发起注定失败的后端往返）", async () => {
    render(<RemoteSection />);
    await waitFor(() => expect(pinInput().value).toBe("4827"));
    fireEvent.change(pinInput(), { target: { value: "13" } });
    expect((screen.getByRole("button", { name: /^save$/i }) as HTMLButtonElement).disabled).toBe(
      true
    );
    expect(invokeMock).not.toHaveBeenCalledWith("remote_set_pin", expect.anything());
    fireEvent.change(pinInput(), { target: { value: "1357" } });
    expect((screen.getByRole("button", { name: /^save$/i }) as HTMLButtonElement).disabled).toBe(
      false
    );
  });

  it("重置设备：二次确认 Dialog——确认调 remote_reset_devices，取消零调用", async () => {
    render(<RemoteSection />);
    await waitFor(() => expect(pinInput().value).toBe("4827"));
    fireEvent.click(screen.getByRole("button", { name: /reset devices/i }));
    expect(await screen.findByText("Reset devices?")).toBeTruthy();
    // 取消：不调后端
    fireEvent.click(screen.getByRole("button", { name: /^cancel$/i }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(invokeMock).not.toHaveBeenCalledWith("remote_reset_devices");
    // 再开 → 确认：remote_reset_devices 被调，Dialog 关
    fireEvent.click(screen.getByRole("button", { name: /reset devices/i }));
    fireEvent.click(await screen.findByRole("button", { name: /reset$/i }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("remote_reset_devices"));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });
});

describe("RemoteSection 已接入设备列表（M5 A6）", () => {
  const devicesOf = () => [
    { id: "d1", name: "JARVIS 的 iPhone", firstPairedAt: 1, lastSeenAt: Date.now(), online: true, via: "quick" },
    { id: "d2", name: "matebook16s · Edge", firstPairedAt: 2, lastSeenAt: Date.now() - 7_200_000, online: false, via: "lan" },
    { id: "d3", name: "Desktop-A", firstPairedAt: 3, lastSeenAt: 3, online: false, via: "local" },
    { id: "d4", name: "iPad", firstPairedAt: 4, lastSeenAt: 4, online: false, via: "named" },
    { id: "d5", name: "Legacy", firstPairedAt: 5, lastSeenAt: 5, online: false },
  ];

  beforeEach(() => {
    invokeMock.mockImplementation(async (cmd: string, args?: { key?: string }) => {
      if (cmd === "remote_status") return statusOf();
      if (cmd === "get_setting") return null;
      if (cmd === "remote_devices") return devicesOf();
      return null;
    });
  });

  it("上限徽标 N / maxDevices——默认 10（决策 #17），后端改上限随之变化（A8：不再硬编码）", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_status") return statusOf({ maxDevices: 7 });
      if (cmd === "get_setting") return null;
      if (cmd === "remote_devices") return devicesOf();
      return null;
    });
    render(<RemoteSection />);
    expect(await screen.findByText("5 / 7")).toBeTruthy();
  });

  it("via 四值徽标映射：quick→临时隧道、lan→局域网、local→本机、named→命名隧道；无 via 不渲染徽标", async () => {
    render(<RemoteSection />);
    const row1 = (await screen.findByText("JARVIS 的 iPhone")).closest("li")!;
    expect(within(row1).getByText("Quick tunnel")).toBeTruthy();
    const row2 = screen.getByText("matebook16s · Edge").closest("li")!;
    expect(within(row2).getByText("LAN")).toBeTruthy();
    const row3 = screen.getByText("Desktop-A").closest("li")!;
    expect(within(row3).getByText("This machine")).toBeTruthy();
    const row4 = screen.getByText("iPad").closest("li")!;
    expect(within(row4).getByText("Named tunnel")).toBeTruthy();
    const row5 = screen.getByText("Legacy").closest("li")!;
    expect(within(row5).queryByText(/tunnel|LAN|machine/i)).toBeNull();
  });

  it("在线点 + 在线/相对时间文案", async () => {
    render(<RemoteSection />);
    const row1 = (await screen.findByText("JARVIS 的 iPhone")).closest("li")!;
    expect(within(row1).getByText("Online")).toBeTruthy();
    expect(within(row1).getByText(/active just now/i)).toBeTruthy();
  });

  it("重命名：点重命名出输入框与保存，保存调 remote_rename_device(id, 新名)", async () => {
    render(<RemoteSection />);
    const row1 = (await screen.findByText("JARVIS 的 iPhone")).closest("li")!;
    fireEvent.click(within(row1).getByRole("button", { name: /rename/i }));
    const input = within(row1).getByLabelText(/rename/i) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "我的手机" } });
    fireEvent.click(within(row1).getByRole("button", { name: /^save$/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("remote_rename_device", {
        id: "d1",
        name: "我的手机",
      })
    );
  });

  it("踢下线：调 remote_revoke_device(id)", async () => {
    render(<RemoteSection />);
    const row2 = (await screen.findByText("matebook16s · Edge")).closest("li")!;
    fireEvent.click(within(row2).getByRole("button", { name: /kick/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("remote_revoke_device", { id: "d2" })
    );
  });

  it("空设备表：渲染占位文案，徽标 0 / 10", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "remote_status") return disabledStatus();
      if (cmd === "get_setting") return null;
      if (cmd === "remote_devices") return [];
      return null;
    });
    render(<RemoteSection />);
    expect(await screen.findByText("No paired devices")).toBeTruthy();
    expect(screen.getByText("0 / 10")).toBeTruthy();
  });
});

describe("RemoteSection 本机名称默认系统名（M5 A6）", () => {
  it("remote.host_name 未设置（null）→ 默认填 remote_status.host.name（系统名），去掉灰字提示", async () => {
    render(<RemoteSection />);
    const input = (await screen.findByLabelText("Machine name")) as HTMLInputElement;
    await waitFor(() => expect(input.value).toBe("matebook16s"));
    // 无占位灰字提示（占位符为空串）
    expect(input.placeholder).toBe("");
    fireEvent.change(input, { target: { value: "my-pc" } });
    fireEvent.blur(input);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("set_setting", {
        key: "remote.host_name",
        value: "my-pc",
      })
    );
  });

  it("已存 host_name 优先于系统名（不回填覆盖用户命名）", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: { key?: string }) => {
      if (cmd === "remote_status") return statusOf();
      if (cmd === "get_setting" && args?.key === "remote.host_name") return "JARVIS-Win";
      if (cmd === "get_setting") return null;
      if (cmd === "remote_devices") return [];
      return null;
    });
    render(<RemoteSection />);
    const input = (await screen.findByLabelText("Machine name")) as HTMLInputElement;
    await waitFor(() => expect(input.value).toBe("JARVIS-Win"));
  });
});

// i18n 契约：RemoteSection / useRemoteEvents 源码里引用的每个 settings.remote.* 键，
// 必须在 zh 与 en 两个 locale 同齐备（缺键即键路径泄漏到 UI）；两 locale 键集必须相等。
describe("RemoteSection i18n zh/en 无缺键（M5 A6）", () => {
  // vitest jsdom 环境下 import.meta.url 非 file 协议，用进程 cwd（vitest 以仓库根启动）
  const root = process.cwd();
  const flat = (obj: Record<string, unknown>, prefix = ""): string[] =>
    Object.entries(obj).flatMap(([k, v]) =>
      typeof v === "object" && v !== null
        ? flat(v as Record<string, unknown>, `${prefix}${k}.`)
        : [`${prefix}${k}`]
    );

  it("源码引用键 zh/en 双语齐备，且两 locale 的 settings.remote 键集相等", () => {
    const zh = JSON.parse(readFileSync(path.join(root, "src/i18n/locales/zh.json"), "utf8"));
    const en = JSON.parse(readFileSync(path.join(root, "src/i18n/locales/en.json"), "utf8"));
    const zhKeys = new Set(
      flat(zh.settings.remote).map((k) => `settings.remote.${k}`)
    );
    const enKeys = new Set(
      flat(en.settings.remote).map((k) => `settings.remote.${k}`)
    );
    // 两 locale 键集相等（对齐 scripts/check-i18n 的子树版）
    expect([...zhKeys].filter((k) => !enKeys.has(k))).toEqual([]);
    expect([...enKeys].filter((k) => !zhKeys.has(k))).toEqual([]);

    // 源码扫描：两个消费方引用的键必须双 locale 存在
    const sources = [
      path.join(root, "src/components/settings/RemoteSection.tsx"),
      path.join(root, "src/hooks/useRemoteEvents.ts"),
    ];
    const used = new Set<string>();
    for (const f of sources) {
      const text = readFileSync(f, "utf8");
      for (const m of text.matchAll(/settings\.remote\.([A-Za-z0-9_]+)/g)) {
        used.add(m[0]);
      }
    }
    expect(used.size).toBeGreaterThan(20);
    const missingZh = [...used].filter((k) => !zhKeys.has(k));
    const missingEn = [...used].filter((k) => !enKeys.has(k));
    expect(missingZh).toEqual([]);
    expect(missingEn).toEqual([]);
  });
});
