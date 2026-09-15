import { describe, expect, it } from "vitest";
import {
  AGENT_TYPES,
  STATUS_DOT_COLOR,
  TOOL_FILTERS,
  TOOL_LABELS,
  filterByAgent,
  formatRelativeTime,
  sortSessions,
  type ToolFilter,
} from "@/mobile/board-logic";
import type { Session, SessionStatus } from "@/types/session";

// 会话夹具：仅 board-logic 消费的字体段有语义，其余给合法默认值
function makeSession(
  overrides: Partial<Session> & Pick<Session, "id" | "status" | "lastActivityAt">
): Session {
  return {
    agentType: "claude",
    projectName: "proj",
    projectPath: "/tmp/proj",
    title: null,
    gitBranch: null,
    githubUrl: null,
    lastMessage: null,
    lastMessageRole: null,
    pid: 1,
    cpuUsage: 0,
    activeSubagentCount: 0,
    form: "cli",
    jumpSupported: false,
    unread: false,
    ...overrides,
  };
}

describe("board-logic 排序（等待 → 运行 → 空闲，同级 lastActivityAt 降序）", () => {
  it("等待卡排在运行卡之前，运行卡排在空闲卡之前", () => {
    const t = "2026-09-14T00:00:00Z";
    const input = [
      makeSession({ id: "idle", status: "idle", lastActivityAt: t }),
      makeSession({ id: "processing", status: "processing", lastActivityAt: t }),
      makeSession({ id: "waiting", status: "waiting", lastActivityAt: t }),
      makeSession({ id: "finished", status: "finished", lastActivityAt: t }),
      makeSession({ id: "thinking", status: "thinking", lastActivityAt: t }),
      makeSession({ id: "compacting", status: "compacting", lastActivityAt: t }),
    ];
    const sorted = sortSessions(input);
    expect(sorted.map((s) => s.id)).toEqual([
      "waiting",
      "processing",
      "thinking",
      "compacting",
      "idle",
      "finished",
    ]);
  });

  it("同级内按 lastActivityAt 降序（最近活跃在前）", () => {
    const input = [
      makeSession({ id: "old", status: "waiting", lastActivityAt: "2026-09-14T00:00:00Z" }),
      makeSession({ id: "new", status: "waiting", lastActivityAt: "2026-09-14T01:00:00Z" }),
      makeSession({ id: "mid", status: "waiting", lastActivityAt: "2026-09-14T00:30:00Z" }),
    ];
    expect(sortSessions(input).map((s) => s.id)).toEqual(["new", "mid", "old"]);
  });

  it("异常时间戳（不可解析）排同级末尾，且不改变原数组", () => {
    const input = [
      makeSession({ id: "bad", status: "idle", lastActivityAt: "not-a-date" }),
      makeSession({ id: "good", status: "idle", lastActivityAt: "2026-09-14T00:00:00Z" }),
    ];
    const sorted = sortSessions(input);
    expect(sorted.map((s) => s.id)).toEqual(["good", "bad"]);
    expect(input.map((s) => s.id)).toEqual(["bad", "good"]);
  });
});

describe("board-logic 工具过滤", () => {
  const input = [
    makeSession({
      id: "c",
      agentType: "claude",
      status: "idle",
      lastActivityAt: "2026-09-14T00:00:00Z",
    }),
    makeSession({
      id: "x",
      agentType: "codex",
      status: "idle",
      lastActivityAt: "2026-09-14T00:00:00Z",
    }),
    makeSession({
      id: "c2",
      agentType: "claude",
      status: "waiting",
      lastActivityAt: "2026-09-14T01:00:00Z",
    }),
  ];

  it("按 agentType 过滤", () => {
    expect(filterByAgent(input, "claude").map((s) => s.id)).toEqual(["c", "c2"]);
    expect(filterByAgent(input, "codex").map((s) => s.id)).toEqual(["x"]);
  });

  it("全部（all）原样返回，不过滤", () => {
    expect(filterByAgent(input, "all")).toHaveLength(3);
  });

  it("chips 顺序 = AgentType 八值顺序 + 全部在前", () => {
    expect(AGENT_TYPES).toEqual([
      "claude",
      "codex",
      "opencode",
      "openclaw",
      "kimi",
      "workbuddy",
      "zcode",
      "dsh",
    ]);
    expect(TOOL_FILTERS).toEqual<ToolFilter[]>([
      "all",
      "claude",
      "codex",
      "opencode",
      "openclaw",
      "kimi",
      "workbuddy",
      "zcode",
      "dsh",
    ]);
  });
});

describe("board-logic 工具显示名（P8c 卡片主行）", () => {
  it("键集穷尽 AgentType 八值（Record 守卫，增删 AgentType 时此断言同步修正）", () => {
    expect(Object.keys(TOOL_LABELS).sort()).toEqual([...AGENT_TYPES].sort());
  });

  it("八值显示名与桌面 agentBadge 口径一致（codex 不分 APP/CLI，统一 Codex）", () => {
    expect(TOOL_LABELS.claude).toBe("Claude");
    expect(TOOL_LABELS.codex).toBe("Codex");
    expect(TOOL_LABELS.opencode).toBe("OpenCode");
    expect(TOOL_LABELS.openclaw).toBe("OpenClaw");
    expect(TOOL_LABELS.kimi).toBe("Kimi Code");
    expect(TOOL_LABELS.workbuddy).toBe("WorkBuddy");
    expect(TOOL_LABELS.zcode).toBe("ZCode");
    expect(TOOL_LABELS.dsh).toBe("DSH");
  });
});

describe("board-logic 状态 → 三色映射（红=等待 / 黄=运行 / 绿=空闲）", () => {
  const cases: Array<[SessionStatus, string]> = [
    ["waiting", "red"],
    ["processing", "yellow"],
    ["thinking", "yellow"],
    ["compacting", "yellow"],
    ["idle", "green"],
    ["finished", "green"],
  ];
  it.each(cases)("%s → %s", (status, color) => {
    expect(STATUS_DOT_COLOR[status]).toContain(color);
  });
});

describe("board-logic 相对时长（now 由调用方注入，可测）", () => {
  const now = Date.parse("2026-09-14T12:00:00Z");

  it("刚发生（<1 分钟）→ 刚刚", () => {
    expect(formatRelativeTime("2026-09-14T11:59:30Z", now)).toBe("刚刚");
  });

  it("小时级 → N 小时前", () => {
    expect(formatRelativeTime("2026-09-14T09:00:00Z", now)).toBe("3 小时前");
  });

  it("天级 → N 天前", () => {
    expect(formatRelativeTime("2026-09-12T00:00:00Z", now)).toBe("2 天前");
  });

  it("分钟级 → N 分钟前", () => {
    expect(formatRelativeTime("2026-09-14T11:33:00Z", now)).toBe("27 分钟前");
  });

  it("未来时间戳（时钟偏差）→ 刚刚", () => {
    expect(formatRelativeTime("2026-09-14T12:05:00Z", now)).toBe("刚刚");
  });

  it("不可解析 → 占位符", () => {
    expect(formatRelativeTime("garbage", now)).toBe("--");
  });
});
