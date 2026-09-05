// tests/session/dismissCard.test.tsx — T2：App 形态卡全部可 X。
// 活跃 App 卡（黄/红）点 X → invoke dismiss_session_card（传当前 status）；
// 未读卡点 X 维持已读语义（mark_session_read）；CLI 卡不显示 X。
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

describe("App 卡 X 关闭（T2）", () => {
  it("活跃 App 卡点 X → invoke dismiss_session_card 且传当前 status", async () => {
    render(<SessionCard session={mk({ status: "waiting", form: "app", unread: false })} />);
    fireEvent.click(screen.getByRole("button", { name: /Hide for now/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("dismiss_session_card", {
        agentType: "workbuddy",
        sessionId: "s1",
        status: "waiting",
      })
    );
  });

  it("未读卡点 X 维持已读语义（mark_session_read）", async () => {
    render(<SessionCard session={mk({ status: "idle", form: "app", unread: true })} />);
    fireEvent.click(screen.getByRole("button", { name: /Mark read/i }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("mark_session_read", {
        agentType: "workbuddy",
        sessionId: "s1",
      })
    );
    expect(invokeMock).not.toHaveBeenCalledWith("dismiss_session_card", expect.anything());
  });

  it("CLI 活跃卡不显示 X（T2 范围仅 App 形态）", () => {
    render(<SessionCard session={mk({ status: "waiting", form: "cli", unread: false })} />);
    expect(screen.queryByRole("button", { name: /Hide for now/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /Mark read/i })).toBeNull();
  });
});
