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
};

// Bug 6（M3 验收）：bind=0.0.0.0 时 remote_status 的 url 与 lanUrls[0] 是同一 IP
// 生成的同一串——设置页去重前渲染两行一模一样的地址
const lanStatus = {
  enabled: true,
  bind: "0.0.0.0",
  port: 9420,
  url: "http://192.168.1.5:9420/m",
  lanUrls: ["http://192.168.1.5:9420/m", "http://10.0.0.2:9420/m"],
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

describe("RemoteSection 访问地址展示（Bug 6 去重）", () => {
  it("lanUrls 含与 url 全等的条目时不重复渲染（url 单独一行展示）", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: { key?: string }) => {
      if (cmd === "remote_status") return { ...lanStatus };
      if (cmd === "get_setting") {
        return args?.key === "remote.host_name" ? "JARVIS-Win" : null;
      }
      return null;
    });
    render(<RemoteSection />);
    // 局域网分区出现（0.0.0.0 绑定 + enabled）
    expect(await screen.findByText("LAN addresses (for the phone)")).toBeTruthy();
    // 与主 url 全等的候选被过滤：整个页面该串只出现一次（访问地址行）
    const dup = screen.getAllByText("http://192.168.1.5:9420/m");
    expect(dup).toHaveLength(1);
    // 其余候选照常展示
    expect(screen.getByText("http://10.0.0.2:9420/m")).toBeTruthy();
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
