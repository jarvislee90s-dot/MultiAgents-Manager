// tests/settings/remoteSection.test.tsx — 评审修复 R2（P8b 收尾）：
// 设置页「本机名称」输入：加载时 get_setting("remote.host_name") 回填当前值，
// blur 时 set_setting 原样落盘（空串语义由后端 display_host_name 过滤，前端不校验）。
// RemoteSection 有既有组件但无既有测试文件，按 toolManagement.test.tsx 的 mock 模式新建。
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

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

void i18n;

const status = {
  enabled: false,
  bind: "127.0.0.1",
  port: 8787,
  url: "http://127.0.0.1:8787/m",
  lanUrls: [],
  addresses: [{ url: "http://127.0.0.1:8787/m", iface: "", primary: true }],
};

// 2026-09-16 用户裁决：地址合并为**一个区块**逐条展示（多网卡有两个不同网段的
// 地址，旧版「访问地址 + 局域网地址」两块并列被读成重复）
const lanStatus = {
  enabled: true,
  bind: "0.0.0.0",
  port: 9420,
  url: "http://192.168.66.202:9420/m",
  lanUrls: ["http://192.168.66.202:9420/m", "http://192.168.42.216:9420/m"],
  addresses: [
    { url: "http://192.168.66.202:9420/m", iface: "WLAN", primary: true },
    { url: "http://192.168.42.216:9420/m", iface: "以太网", primary: false },
  ],
};

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (cmd: string, args?: { key?: string }) => {
    if (cmd === "remote_status") return { ...status };
    if (cmd === "get_setting") {
      return args?.key === "remote.host_name" ? "JARVIS-Win" : null;
    }
    return null;
  });
});

describe("RemoteSection 访问地址展示（2026-09-16 单区块 + 网卡名）", () => {
  it("多网卡地址在同一个「访问地址」区块逐条展示，各带网卡名与推荐标记", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: { key?: string }) => {
      if (cmd === "remote_status") return { ...lanStatus };
      if (cmd === "get_setting") {
        return args?.key === "remote.host_name" ? "JARVIS-Win" : null;
      }
      return null;
    });
    render(<RemoteSection />);
    // 单区块：地址标签只出现一次（旧版是「访问地址」+「局域网地址」两块并列），
    // 且不再有独立的局域网地址分区
    expect(await screen.findAllByText("Address")).toHaveLength(1);
    expect(screen.queryByText("LAN addresses (for the phone)")).toBeNull();
    // 两个地址都在，且逐条标注所属网卡
    expect(screen.getByText("http://192.168.66.202:9420/m")).toBeTruthy();
    expect(screen.getByText("http://192.168.42.216:9420/m")).toBeTruthy();
    expect(screen.getByText("WLAN")).toBeTruthy();
    expect(screen.getByText("以太网")).toBeTruthy();
    // 主（推荐）地址有标记，另一条没有
    expect(screen.getAllByText("Recommended")).toHaveLength(1);
  });

  it("loopback 绑定：单条目（本机），无网卡名与推荐噪声", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: { key?: string }) => {
      if (cmd === "remote_status") {
        return {
          enabled: true,
          bind: "127.0.0.1",
          port: 9420,
          url: "http://127.0.0.1:9420/m",
          lanUrls: [],
          addresses: [{ url: "http://127.0.0.1:9420/m", iface: "本机", primary: true }],
        };
      }
      if (cmd === "get_setting") {
        return args?.key === "remote.host_name" ? "JARVIS-Win" : null;
      }
      return null;
    });
    render(<RemoteSection />);
    expect(await screen.findAllByText("Address")).toHaveLength(1);
    expect(screen.getByText("http://127.0.0.1:9420/m")).toBeTruthy();
    expect(screen.getByText("本机")).toBeTruthy();
  });
});

describe("RemoteSection 本机名称输入（P8b 收尾）", () => {
  it("加载时回填当前值：get_setting(remote.host_name) → 输入框显示已存名称", async () => {
    render(<RemoteSection />);
    const input = (await screen.findByLabelText("Machine name")) as HTMLInputElement;
    await waitFor(() => expect(input.value).toBe("JARVIS-Win"));
  });

  it("输入 + blur → set_setting 以 (remote.host_name, 新值) 落盘（原样写，不做非空校验）", async () => {
    render(<RemoteSection />);
    const input = (await screen.findByLabelText("Machine name")) as HTMLInputElement;
    await waitFor(() => expect(input.value).toBe("JARVIS-Win"));
    fireEvent.change(input, { target: { value: "My PC" } });
    fireEvent.blur(input);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("set_setting", {
        key: "remote.host_name",
        value: "My PC",
      })
    );
  });

  it("加载后未改动即 blur：同样落盘当前值（幂等写，后端 KV 原语义）", async () => {
    render(<RemoteSection />);
    const input = (await screen.findByLabelText("Machine name")) as HTMLInputElement;
    await waitFor(() => expect(input.value).toBe("JARVIS-Win"));
    fireEvent.blur(input);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("set_setting", {
        key: "remote.host_name",
        value: "JARVIS-Win",
      })
    );
  });
});
