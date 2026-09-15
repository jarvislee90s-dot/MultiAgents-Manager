import { describe, expect, it } from "vitest";
import {
  AGENT_TYPES,
  CHIP_LIGHT_TEXT_FACTOR,
  STATUS_DOT_COLOR,
  STATUS_LABELS,
  TOOL_BRAND_COLORS,
  TOOL_FILTERS,
  TOOL_LABELS,
  applyTransition,
  darkenHex,
  filterByAgent,
  filterEnabledTools,
  formatRelativeTime,
  formatTransition,
  sortChipsByActivity,
  sortSessions,
  type ToolFilter,
} from "@/mobile/board-logic";
import type { Session, SessionStatus, TransitionEvent } from "@/types/session";

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

// M3 Task 6：跃迁事件 → 展示层纯函数（横幅文案 / 看板卡应用）
describe("board-logic 跃迁展示（M3 Task 6）", () => {
  function makeTransition(overrides: Partial<TransitionEvent> = {}): TransitionEvent {
    return {
      sessionId: "s1",
      agentType: "claude",
      from: "processing",
      to: "waiting",
      projectName: "mam",
      lastMessage: null,
      ts: 42,
      ...overrides,
    };
  }

  it("STATUS_LABELS 键集穷尽 SessionStatus 六值（横幅不会静默缺文案）", () => {
    expect(Object.keys(STATUS_LABELS).sort()).toEqual([
      "compacting",
      "finished",
      "idle",
      "processing",
      "thinking",
      "waiting",
    ]);
  });

  it("formatTransition：工具 · 项目 · 前态 → 后态 [· 消息预览]", () => {
    expect(formatTransition(makeTransition())).toBe("Claude · mam · 运行中 → 等待操作");
    expect(formatTransition(makeTransition({ lastMessage: "需要批准" }))).toBe(
      "Claude · mam · 运行中 → 等待操作 · 需要批准"
    );
    // lastMessage 空串与 null 同样省略预览段（不渲染孤立的 ` · `）
    expect(formatTransition(makeTransition({ lastMessage: "" }))).toBe(
      "Claude · mam · 运行中 → 等待操作"
    );
  });

  it("formatTransition：未知 wire 值回落原样字符串（JSON.parse 结果不可信，不渲染 undefined）", () => {
    const weird = makeTransition({
      agentType: "future-tool" as TransitionEvent["agentType"],
      from: "weird" as TransitionEvent["from"],
      to: "state" as TransitionEvent["to"],
    });
    expect(formatTransition(weird)).toBe("future-tool · mam · weird → state");
  });

  it("applyTransition：命中 (工具, 会话 id) 时更新 status/lastMessage，返回新数组且不改入参", () => {
    const before = makeSession({ id: "s1", status: "processing", lastActivityAt: "t" });
    const input = [before];
    const out = applyTransition(input, makeTransition({ lastMessage: "done" }));
    expect(out).not.toBe(input);
    expect(out[0]).not.toBe(before); // 对象也是新引用（不可变更新）
    expect(out[0].status).toBe("waiting");
    expect(out[0].lastMessage).toBe("done");
    expect(input[0].status).toBe("processing"); // 入参未被改
    expect(before.lastMessage).toBeNull();
  });

  it("applyTransition：跨工具撞 id 不误命中（键取 (工具, id) 二元组，同 watcher diff 口径）", () => {
    const input = [
      makeSession({ id: "shared", agentType: "claude", status: "idle", lastActivityAt: "t" }),
      makeSession({ id: "shared", agentType: "codex", status: "idle", lastActivityAt: "t" }),
    ];
    const out = applyTransition(input, makeTransition({ sessionId: "shared", agentType: "codex" }));
    expect(out[0].status).toBe("idle"); // claude 的同名会话不受影响
    expect(out[1].status).toBe("waiting");
  });

  it("applyTransition：未命中（新会话 / 已消失 / 坏状态串）原样返回同一引用（零多余渲染）", () => {
    const input = [makeSession({ id: "s1", status: "idle", lastActivityAt: "t" })];
    // 目标会话不在当前列表（事件早于快照到达 / 卡已消失）
    expect(applyTransition(input, makeTransition({ sessionId: "gone" }))).toBe(input);
    // 未知状态串（wire 损坏）不写入：状态点取色会渲染出 undefined 类名
    const bad = makeTransition({ to: "bogus" as TransitionEvent["to"] });
    expect(applyTransition(input, bad)).toBe(input);
    expect(input[0].status).toBe("idle");
  });
});

// P8d/P8e（M3 Task 3）：受管过滤 + 品牌色 chips + 活跃排序
describe("P8d/P8e board-logic", () => {
  it("品牌色映射覆盖八工具", () => {
    for (const t of [
      "claude",
      "codex",
      "opencode",
      "openclaw",
      "kimi",
      "workbuddy",
      "zcode",
      "dsh",
    ]) {
      expect(TOOL_BRAND_COLORS[t]).toBeTruthy();
    }
  });

  it("品牌色键集穷尽 AgentType 八值（Record 守卫，增删 AgentType 时此断言同步修正）", () => {
    expect(Object.keys(TOOL_BRAND_COLORS).sort()).toEqual([...AGENT_TYPES].sort());
  });

  it("按最新活跃排序（zcode 12:00 最新在前）", () => {
    const sessions = [
      { agentType: "claude", lastActivityAt: "2026-09-15T10:00:00Z" },
      { agentType: "zcode", lastActivityAt: "2026-09-15T12:00:00Z" },
      { agentType: "claude", lastActivityAt: "2026-09-15T11:00:00Z" },
    ];
    const sorted = sortChipsByActivity(["claude", "zcode"], sessions);
    expect(sorted[0]).toBe("zcode"); // 12:00 最新
    expect(sorted[1]).toBe("claude");
  });

  it("无会话工具排末尾（有活跃时间的工具在前，相对顺序稳定）", () => {
    const sessions = [{ agentType: "codex", lastActivityAt: "2026-09-15T08:00:00Z" }];
    const sorted = sortChipsByActivity(["zcode", "codex", "claude"], sessions);
    expect(sorted).toEqual(["codex", "zcode", "claude"]);
  });

  it("不可解析时间戳按 0 处理（NaN 防御：坏时间不压过无会话工具）", () => {
    const sessions = [
      { agentType: "codex", lastActivityAt: "not-a-date" },
      { agentType: "claude", lastActivityAt: "2026-09-15T08:00:00Z" },
    ];
    const sorted = sortChipsByActivity(["codex", "claude", "zcode"], sessions);
    expect(sorted).toEqual(["claude", "codex", "zcode"]);
  });

  it("仅显示受管工具 ∩ 有卡工具", () => {
    const result = filterEnabledTools(["claude", "codex", "zcode"], new Set(["claude", "zcode"]));
    expect(result).toEqual(["claude", "zcode"]);
    expect(result).not.toContain("codex");
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

// P8f chip 浅色态配色（Task 4）：品牌原色当文字在浅底上对比度不足（实测 1.96–3.84），
// 压暗到 AA 后交付。本组测试把「系数口径」与「八色全达标」锁进 CI——
// 未来有人替换品牌色/放大系数导致不达标时立刻变红。
describe("P8f chip 浅色态文字色（darkenHex + 对比度）", () => {
  // WCAG 相对亮度与对比度（2.0 版公式，与实现文档中的实测口径一致）
  function luminance(hex: string): number {
    const [r, g, b] = [1, 3, 5]
      .map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
      .map((v) => (v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4)));
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  }
  function contrast(a: string, b: string): number {
    const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
    return (hi + 0.05) / (lo + 0.05);
  }
  // 12% 品牌色淡底（chip 未选中态背景；白底 alpha 合成）
  function blendedBackground(brand: string): string {
    const rgb = [1, 3, 5].map((i) => parseInt(brand.slice(i, i + 2), 16));
    return (
      "#" +
      rgb
        .map((v) =>
          Math.round(v * 0.12 + 255 * 0.88)
            .toString(16)
            .padStart(2, "0")
        )
        .join("")
    );
  }

  it("darkenHex：各通道乘系数并取整（含进位/截断边界）", () => {
    expect(darkenHex("#ffffff", 0.6)).toBe("#999999");
    expect(darkenHex("#000000", 0.6)).toBe("#000000");
    expect(darkenHex("#D97757", 0.6)).toBe("#824734");
    // 四舍五入边界：0.6 × 255 = 153 → 99(h)
    expect(darkenHex("#ff0000", 0.5)).toBe("#800000"); // 0.5×255=127.5 → 128
  });

  it("darkenHex：非法入参原样返回（不产出 NaN 色）", () => {
    expect(darkenHex("not-a-color", 0.6)).toBe("not-a-color");
    expect(darkenHex("#fff", 0.6)).toBe("#fff"); // 3 位简写不接受，避免歧义
    expect(darkenHex("", 0.6)).toBe("");
  });

  it("八工具品牌色压暗后，在白底 + 12% 品牌底上对比度全部 ≥ 4.5（WCAG AA 小字）", () => {
    for (const brand of Object.values(TOOL_BRAND_COLORS)) {
      const text = darkenHex(brand, CHIP_LIGHT_TEXT_FACTOR);
      const bg = blendedBackground(brand);
      const c = contrast(text, bg);
      expect(
        c,
        `品牌色 ${brand} 压暗后 ${text} 在 ${bg} 上对比度仅 ${c.toFixed(2)}`
      ).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("回归锁：品牌原色在浅底上确实不达标（证明压暗不是多余变换）", () => {
    const failures = Object.values(TOOL_BRAND_COLORS).filter(
      (brand) => contrast(brand, blendedBackground(brand)) < 4.5
    );
    // 八色全部不达标——若未来品牌色整体换深色系，此断言会变红，届时可评估移除压暗逻辑
    expect(failures).toHaveLength(8);
  });
});
