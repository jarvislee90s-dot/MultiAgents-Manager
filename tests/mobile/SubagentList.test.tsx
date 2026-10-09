import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SubagentList from "@/mobile/SubagentList";
import type { SubagentView } from "@/mobile/api";

// 观察台 §二：单一列表派发序（服务端序即派发序）/ 绿灰点 / 冻结值 /
// 点击回调 / 空（null/[]）整体不渲染。

function sa(
  id: string,
  name: string,
  status: "running" | "idle",
  spawnTs: string | null,
  endTs: string | null,
  description: string | null = "设计新建会话两改动实现方案"
): SubagentView {
  return {
    id,
    name,
    status,
    spawnTs,
    endTs,
    description,
    tokens: { input: 5124, cacheRead: 56448, cacheCreation: 0, output: 9312 },
  };
}

describe("SubagentList（文件面板子 Agent 卡区）", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("空（null / []）整体不渲染（§二.7）", () => {
    const { container, rerender } = render(<SubagentList list={null} onOpen={() => {}} />);
    expect(container.querySelector('[data-testid="subagent-list"]')).toBeNull();
    rerender(<SubagentList list={[]} onOpen={() => {}} />);
    expect(container.querySelector('[data-testid="subagent-list"]')).toBeNull();
  });

  it("四要素 + 绿点走字 / 灰点冻结原位（§二.2/§二.3）", async () => {
    vi.setSystemTime(new Date("2026-10-08T07:36:45Z"));
    const list = [
      sa("a1", "Plan", "running", "2026-10-08T07:31:07Z", null), // 5:38 走字
      sa("b1", "Explore", "idle", "2026-10-08T07:00:00Z", "2026-10-08T07:30:00Z"), // 冻结 30:00
    ];
    render(<SubagentList list={list} onOpen={() => {}} />);
    expect(screen.getByTestId("subagent-card-a1").textContent).toContain("Plan");
    expect(screen.getByTestId("subagent-card-a1").textContent).toContain("5:38");
    expect(screen.getByTestId("subagent-card-a1").textContent).toContain("7.09万");
    expect(screen.getByTestId("subagent-dot-a1").className).toContain("bg-emerald-500");
    expect(screen.getByTestId("subagent-card-b1").textContent).toContain("30:00");
    expect(screen.getByTestId("subagent-dot-b1").className).toContain("bg-gray-400");
    // 灰点冻结：时钟前进 60s，b1 不动、a1 继续走（Async 版：等 React 调度刷新 DOM——
    // 同步 advance 不让出宏任务队列，重渲染不落地；与 SubagentChips 自走用例同法）
    await vi.advanceTimersByTimeAsync(60_000);
    expect(screen.getByTestId("subagent-card-b1").textContent).toContain("30:00");
    expect(screen.getByTestId("subagent-card-a1").textContent).toContain("6:38");
  });

  it("列表不重排：服务端序即派发序，续跑灰→绿原位（§二.1/§二.3）", () => {
    vi.setSystemTime(new Date("2026-10-08T07:36:45Z"));
    const list = [
      sa("a1", "Plan", "idle", "2026-10-08T07:00:00Z", "2026-10-08T07:10:00Z"),
      sa("b1", "Explore", "running", "2026-10-08T07:31:00Z", null),
    ];
    const { rerender } = render(<SubagentList list={list} onOpen={() => {}} />);
    const cards = () => screen.getByTestId("subagent-list").querySelectorAll("li");
    expect(cards()[0].querySelector('[data-testid="subagent-card-a1"]')).toBeTruthy();
    // a1 续跑：同位置灰→绿，仍在 b1 之前
    const resumed = [{ ...list[0], status: "running" as const, endTs: null }, list[1]];
    rerender(<SubagentList list={resumed} onOpen={() => {}} />);
    expect(cards()[0].querySelector('[data-testid="subagent-card-a1"]')).toBeTruthy();
    expect(screen.getByTestId("subagent-dot-a1").className).toContain("bg-emerald-500");
    // 时长连续累计不归零（07:00 起，now=07:36:45 → 36:45）
    expect(screen.getByTestId("subagent-card-a1").textContent).toContain("36:45");
  });

  it("点击卡片（绿/灰皆可）→ onOpen(id)（§二.5）", () => {
    vi.setSystemTime(new Date("2026-10-08T07:36:45Z"));
    const onOpen = vi.fn();
    render(
      <SubagentList
        list={[
          sa("a1", "Plan", "running", "2026-10-08T07:31:07Z", null),
          sa("b1", "Explore", "idle", "2026-10-08T07:00:00Z", "2026-10-08T07:30:00Z"),
        ]}
        onOpen={onOpen}
      />
    );
    fireEvent.click(screen.getByTestId("subagent-card-a1"));
    fireEvent.click(screen.getByTestId("subagent-card-b1"));
    expect(onOpen.mock.calls.map((c) => c[0])).toEqual(["a1", "b1"]);
  });
});
