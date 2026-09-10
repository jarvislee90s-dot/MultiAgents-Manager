import { describe, it, expect } from "vitest";
import { sessionTitleOrUndefined } from "@/lib/sessionTitle";
import { pairingAmbiguitySet, pairingKey } from "@/lib/pairing";
import type { Session } from "@/types/session";

// issue #45：Session 来源的 title 空传语义收敛契约——null/空串/全空白 → undefined
describe("sessionTitleOrUndefined", () => {
  it("keeps non-empty title as-is", () => {
    expect(sessionTitleOrUndefined({ title: "（1）使用技能【convert】" })).toBe(
      "（1）使用技能【convert】"
    );
  });

  it("normalizes null / empty / whitespace-only to undefined", () => {
    expect(sessionTitleOrUndefined({ title: null })).toBeUndefined();
    expect(sessionTitleOrUndefined({ title: "" })).toBeUndefined();
    expect(sessionTitleOrUndefined({ title: "   " })).toBeUndefined();
  });
});

function fakeSession(over: Partial<Session>): Session {
  return {
    id: "s1",
    agentType: "kimi",
    projectName: "proj",
    projectPath: "/tmp/proj",
    title: null,
    gitBranch: null,
    githubUrl: null,
    status: "processing",
    lastMessage: null,
    lastMessageRole: null,
    lastActivityAt: "2026-09-11T00:00:00Z",
    pid: 1,
    cpuUsage: 0,
    activeSubagentCount: 0,
    form: "cli",
    jumpSupported: true,
    unread: false,
    ...over,
  } as Session;
}

// issue #48：同工具同项目（忽略大小写）≥2 会话 → 配对不确定
describe("pairingAmbiguitySet", () => {
  it("marks sessions sharing tool+project (case-insensitive)", () => {
    const sessions = [
      fakeSession({ id: "a", agentType: "kimi", projectName: "Proj-X" }),
      fakeSession({ id: "b", agentType: "kimi", projectName: "proj-x" }),
      fakeSession({ id: "c", agentType: "opencode", projectName: "proj-x" }),
    ];
    const set = pairingAmbiguitySet(sessions);
    expect(set.has(pairingKey(sessions[0]))).toBe(true);
    expect(set.has(pairingKey(sessions[1]))).toBe(true);
    // 同项目但不同工具：不受影响
    expect(set.has(pairingKey(sessions[2]))).toBe(false);
  });

  it("single session or empty project name is never ambiguous", () => {
    const single = [fakeSession({ projectName: "solo" })];
    expect(pairingAmbiguitySet(single).size).toBe(0);
    const empty = [
      fakeSession({ id: "a", projectName: "" }),
      fakeSession({ id: "b", projectName: "" }),
    ];
    expect(pairingAmbiguitySet(empty).size).toBe(0);
  });
});
