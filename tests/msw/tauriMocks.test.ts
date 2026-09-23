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
});
