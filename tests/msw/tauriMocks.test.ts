import { invoke } from "@tauri-apps/api/core";
import { describe, it, expect } from "vitest";

describe("Tauri mock", () => {
  it("mocks get_all_sessions with correct shape", async () => {
    const result = await invoke("get_all_sessions") as { sessions: unknown[]; totalCount: number; waitingCount: number };
    expect(result).toHaveProperty("sessions");
    expect(result.sessions).toHaveLength(2);
    expect(result.totalCount).toBe(2);
    expect(result.waitingCount).toBe(1);
  });

  it("mocks list_presets", async () => {
    const result = await invoke("list_presets") as Array<{ id: string; name: string }>;
    expect(result).toHaveLength(1);
    expect(result[0].name).toBe("前端开发");
  });

  it("mocks unknown commands with undefined", async () => {
    const result = await invoke("unknown_command");
    expect(result).toBeUndefined();
  });
});
