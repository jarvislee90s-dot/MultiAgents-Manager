import { invoke } from "@tauri-apps/api/core";
import { describe, it, expect } from "vitest";

describe("Tauri mock", () => {
  it("mocks get_all_sessions with correct shape", async () => {
    const result = (await invoke("get_all_sessions")) as {
      sessions: unknown[];
      totalCount: number;
      waitingCount: number;
    };
    expect(result).toHaveProperty("sessions");
    expect(result.sessions).toHaveLength(2);
    expect(result.totalCount).toBe(2);
    expect(result.waitingCount).toBe(1);
  });

  // 预设组 v2（M2 前端基建）：fixture 固定为 1 通用 + 1 tool 私有样例（mock parity 收口门禁）
  it("mocks list_presets with v2 shape", async () => {
    const result = (await invoke("list_presets")) as Array<{
      id: string;
      name: string;
      description: string;
      scope: string;
      boundTool: string | null;
    }>;
    expect(result).toHaveLength(2);
    expect(result[0]).toMatchObject({ name: "前端开发", scope: "universal", boundTool: null });
    expect(result[1]).toMatchObject({ scope: "tool", boundTool: "claude" });
  });

  it("mocks unknown commands with undefined", async () => {
    const result = await invoke("unknown_command");
    expect(result).toBeUndefined();
  });

  // 用量域（计划①）：6 条命令的 mock 形状必须与 Rust 值对象（serde camelCase）逐字一致
  it("mocks usage_dashboard with contract shape", async () => {
    const d = (await invoke("usage_dashboard", {
      range: { preset: "today" },
      groupBy: "tool",
    })) as Record<string, unknown>;
    expect(d).toMatchObject({
      groupBy: "tool",
      hero: expect.any(Number),
      collectedAt: expect.any(Number),
    });
    expect(Array.isArray(d.rows)).toBe(true);
    expect(Array.isArray(d.trend)).toBe(true);
    expect(Array.isArray(d.availability)).toBe(true);
    // compare / recentSession 契约是 `| null`：mock 必须**显式**给值（不是 undefined）。
    // **Minor 2**：`toHaveProperty` 对 `{compare: undefined}` 也过，故这里直接钉值。
    expect(d.compare).toBeNull();
    expect(d.recentSession === null || typeof d.recentSession === "object").toBe(true);
    const ws = d.workSummary as Record<string, unknown>;
    // 不可得字段是 null，不是 0（D15/D16/D19）；同样不得是 undefined
    expect(ws.sessions).not.toBeUndefined();
    expect(ws.topTool).not.toBeUndefined();
    expect(ws.longestTurnPerTool).not.toBeUndefined();
  });

  it("mocks usage_records / usage_export_csv / usage settings", async () => {
    const r = (await invoke("usage_records", {
      range: { preset: "today" },
      // W6：记录页只接受 tool | project（provider/model 会返回 usage-groupby-invalid）
      groupBy: "project",
      filters: {},
    })) as { cards: unknown[]; collectedAt: number };
    expect(Array.isArray(r.cards)).toBe(true);
    const csv = (await invoke("usage_export_csv", {
      range: { preset: "today" },
      groupBy: "tool",
      filters: {},
    })) as string;
    expect(csv.split("\n")[0]).toContain("groupKey");
    const s = (await invoke("usage_set_settings", { patch: { detailRetentionDays: 30 } })) as {
      detailRetentionDays: number;
    };
    expect(s.detailRetentionDays).toBe(30);
  });

  // 采集结果样例固定为 7 源且含 1 个失败源：设置页「用量采集状态」（Task 24）的目验入口
  it("mocks usage_collect with 7 sources including one failure", async () => {
    const c = (await invoke("usage_collect", { force: true })) as {
      sources: { sourceId: string; ok: boolean; errorCode?: string }[];
      totalNewRecords: number;
      durationMs: number;
    };
    expect(c.sources).toHaveLength(7);
    expect(c.sources.filter((x) => !x.ok)).toHaveLength(1);
    expect(c.sources.find((x) => !x.ok)?.errorCode).toBe("usage-source-db-open");
    expect(c.totalNewRecords).toBeGreaterThan(0);
    expect(c.durationMs).toBeGreaterThan(0);
  });
});
