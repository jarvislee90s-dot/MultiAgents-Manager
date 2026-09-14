// tests/remote/api.test.ts — Task 7：远程命令四封装的锁形测试。
// 命令名与参数形态是前端 ↔ Rust（src-tauri/src/remote/mod.rs #[tauri::command]）的
// 隐式契约：字符串漂移后 invoke 只会静默 resolve undefined，不报错——必须在此锁定。
// mock 模式仿 tests/session/dismissCard.test.tsx（vi.hoisted + vi.mock，自带一份，
// 不依赖 setup.ts 全局 tauriInvokeMock 的 default 分支）。
import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

import {
  remoteConfirmPublic,
  remoteIssueToken,
  remoteStatus,
  remoteToggle,
} from "@/lib/api/remote";

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

describe("remote api wrappers", () => {
  it("remoteStatus 无参调用 remote_status，透传返回值", async () => {
    const status = {
      enabled: true,
      bind: "0.0.0.0",
      port: 9420,
      url: "http://0.0.0.0:9420/m",
      lanUrls: ["http://192.168.1.5:9420/m"],
    };
    invokeMock.mockResolvedValue(status);
    expect(await remoteStatus()).toEqual(status);
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith("remote_status");
  });

  it("remoteToggle 以 { enabled } 形态传参（Rust 端 enabled: bool）", async () => {
    await remoteToggle(true);
    expect(invokeMock).toHaveBeenCalledWith("remote_toggle", { enabled: true });
    await remoteToggle(false);
    expect(invokeMock).toHaveBeenCalledWith("remote_toggle", { enabled: false });
    expect(invokeMock).toHaveBeenCalledTimes(2);
  });

  it("remoteIssueToken 无参调用，透传 token/url", async () => {
    const tok = { token: "tok-x", url: "http://127.0.0.1:9420/m#token=tok-x" };
    invokeMock.mockResolvedValue(tok);
    expect(await remoteIssueToken()).toEqual(tok);
    expect(invokeMock).toHaveBeenCalledWith("remote_issue_token");
  });

  it("remoteConfirmPublic 无参调用 remote_confirm_public", async () => {
    await remoteConfirmPublic();
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith("remote_confirm_public");
  });
});
