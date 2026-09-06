// tests/session/sessionCardHeader.test.tsx — Task 6：APP 类卡片头部重排（全工具统一）。
// 顺序：标签→目录名→总结/hash→分支 | 状态指示区 | X 最右；
// 未读与状态灯合并为单一指示（光环徽章，不再渲染独立小绿点）；X 仍 stopPropagation。
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("sonner", () => ({ toast: { info: vi.fn(), error: vi.fn(), success: vi.fn() } }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
// tests/setup.ts 未初始化 i18n，显式引入（fallbackLng=en，与断言文案一致）
import i18n from "@/i18n";
import { SessionCard } from "@/components/sessions/SessionCard";
import type { Session } from "@/types/session";

void i18n;

const mk = (over: Partial<Session>): Session => ({
  id: "s1",
  agentType: "workbuddy",
  projectName: "项目A",
  projectPath: "/a",
  title: null,
  gitBranch: null,
  githubUrl: null,
  status: "waiting",
  lastMessage: "运行中",
  lastMessageRole: null,
  lastActivityAt: "",
  pid: 42,
  cpuUsage: 0,
  activeSubagentCount: 0,
  form: "app",
  jumpSupported: true,
  unread: false,
  ...over,
});

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

/** 头部行：唯一同时带 mb-2 与 flex 的行（中间段落无 flex，底部行无 mb-2） */
const headerRow = (container: HTMLElement) => container.querySelector(".mb-2.flex")!;

describe("APP 卡头部重排（Task 6）", () => {
  it("顺序：标签→目录名→总结/hash→分支 | 状态区 | X 最右", () => {
    const { container } = render(
      <SessionCard
        session={mk({
          status: "idle",
          form: "app",
          unread: false,
          title: "总结标题",
          gitBranch: "main",
        })}
      />
    );
    const header = headerRow(container);
    // 左侧信息组：工具标签 → 项目目录名 → 总结/hash → 分支
    const left = header.firstElementChild!;
    const texts = Array.from(left.children).map((el) => el.textContent);
    expect(texts).toEqual(["WorkBuddy", "项目A", "总结标题", "main"]);
    // 右侧：状态指示区 + 关闭 X（X 位于整行最右）
    const right = header.lastElementChild!;
    expect(right.children.length).toBe(2);
    expect(right.lastElementChild!.tagName).toBe("BUTTON");
    expect(right.lastElementChild!.getAttribute("aria-label")).toContain("Hide for now");
  });

  it("无总结时回退 8 位 hash，且 hash 位于目录名之后", () => {
    const { container } = render(
      <SessionCard session={mk({ id: "abcdefgh1234", title: null, form: "app" })} />
    );
    const left = headerRow(container).firstElementChild!;
    const texts = Array.from(left.children).map((el) => el.textContent);
    expect(texts).toEqual(["WorkBuddy", "项目A", "abcdefgh"]);
  });

  it("未读与状态灯合并为单一指示：无独立小绿点，未读以光环徽章表达", () => {
    const { container } = render(
      <SessionCard session={mk({ status: "idle", form: "app", unread: true })} />
    );
    // 旧独立绿点（bg-emerald-400 圆点）不再渲染
    expect(container.querySelector(".bg-emerald-400")).toBeNull();
    // 未读光环徽章存在（aria-label=Unread），且状态灯仍在
    const ring = screen.getByLabelText("Unread");
    expect(ring.className).toContain("ring-emerald-400/80");
    expect(container.querySelector(".bg-green-500")).not.toBeNull();
    // 未读指示唯一：全卡仅一个 Unread 标签
    expect(screen.getAllByLabelText("Unread")).toHaveLength(1);
  });

  it("已读卡不显示未读光环", () => {
    render(<SessionCard session={mk({ status: "idle", form: "app", unread: false })} />);
    expect(screen.queryByLabelText("Unread")).toBeNull();
  });

  it("未读卡点 X：标记已读且不触发卡片跳转（stopPropagation）", async () => {
    render(<SessionCard session={mk({ status: "idle", form: "app", unread: true })} />);
    fireEvent.click(screen.getByRole("button", { name: /Mark read/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("mark_session_read", {
        agentType: "workbuddy",
        sessionId: "s1",
      })
    );
    expect(invokeMock).not.toHaveBeenCalledWith("focus_session", expect.anything());
  });

  it("活跃 App 卡点 X：dismiss 且不触发卡片跳转（stopPropagation）", async () => {
    render(<SessionCard session={mk({ status: "waiting", form: "app", unread: false })} />);
    fireEvent.click(screen.getByRole("button", { name: /Hide for now/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("dismiss_session_card", {
        agentType: "workbuddy",
        sessionId: "s1",
        status: "waiting",
      })
    );
    expect(invokeMock).not.toHaveBeenCalledWith("focus_session", expect.anything());
  });

  it("CLI 卡不回归：无 X、无未读光环，状态灯正常", () => {
    const { container } = render(
      <SessionCard session={mk({ status: "processing", form: "cli", unread: false })} />
    );
    expect(screen.queryByRole("button", { name: /Hide for now/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /Mark read/i })).toBeNull();
    expect(screen.queryByLabelText("Unread")).toBeNull();
    // 状态灯仍在（黄灯 = 运行中）
    expect(container.querySelector(".bg-yellow-500")).not.toBeNull();
  });

  it("CLI 卡未读语义保留：光环显示，X 走已读（未读为 APP 专用，防御性路径不变）", async () => {
    render(<SessionCard session={mk({ status: "idle", form: "cli", unread: true })} />);
    expect(screen.getByLabelText("Unread")).not.toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /Mark read/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("mark_session_read", {
        agentType: "workbuddy",
        sessionId: "s1",
      })
    );
  });
});
