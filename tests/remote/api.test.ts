// tests/remote/api.test.ts — M5 A6：远程命令封装的锁形测试（重写后口径）。
// 命令名与参数形态是前端 ↔ Rust（src-tauri/src/remote/mod.rs #[tauri::command]）的
// 隐式契约：字符串漂移后 invoke 只会静默 resolve undefined，不报错——必须在此锁定。
// M5 A3 起 /pair/pin 认证制下线审批/直通命令：remote_issue_token / remote_pending_requests /
// remote_approve_request / remote_set_channel 的封装已随 A6 删除，此处不再出现。
// mock 模式仿 tests/session/dismissCard.test.tsx（vi.hoisted + vi.mock，自带一份，
// 不依赖 setup.ts 全局 tauriInvokeMock 的 default 分支）。
import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

import {
  remoteConfirmPublic,
  remoteDevices,
  remoteRevokeAllDevices,
  remoteRevokeDevice,
  remoteStatus,
  remoteToggle,
  renameDevice,
  resetDevices,
  setPin,
  toggleChannel,
} from "@/lib/api/remote";

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

describe("remote api wrappers", () => {
  it("remoteStatus 无参调用 remote_status，透传返回值", async () => {
    const status = {
      enabled: true,
      maxDevices: 10,
      channels: {
        local: { running: true, address: "http://127.0.0.1:9420/m" },
        lan: { enabled: true, running: true, addresses: ["http://192.168.1.5:9420/m"] },
        quick: { enabled: false, running: false, address: null, error: null },
        named: { enabled: false, running: false, address: null, error: null },
      },
      pin: "4827",
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

  it("toggleChannel 以 (channel, on) 字面形态传参（remote_toggle_channel(channel, on)）", async () => {
    await toggleChannel("lan", true);
    expect(invokeMock).toHaveBeenCalledWith("remote_toggle_channel", { channel: "lan", on: true });
    await toggleChannel("quick", false);
    expect(invokeMock).toHaveBeenCalledWith("remote_toggle_channel", { channel: "quick", on: false });
    await toggleChannel("named", true);
    expect(invokeMock).toHaveBeenCalledWith("remote_toggle_channel", { channel: "named", on: true });
  });

  it("setPin 以 { pin } 形态传参（remote_set_pin(pin: String)）", async () => {
    await setPin("4827");
    expect(invokeMock).toHaveBeenCalledWith("remote_set_pin", { pin: "4827" });
  });

  it("resetDevices 无参调用 remote_reset_devices（吊销全部，不改 PIN）", async () => {
    await resetDevices();
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith("remote_reset_devices");
  });

  it("renameDevice 以 { id, name } 形态传参（remote_rename_device(id, name)）", async () => {
    await renameDevice("d1", "我的手机");
    expect(invokeMock).toHaveBeenCalledWith("remote_rename_device", { id: "d1", name: "我的手机" });
  });

  it("remoteConfirmPublic 无参调用 remote_confirm_public", async () => {
    await remoteConfirmPublic();
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith("remote_confirm_public");
  });

  it("设备命令三封装：花名册 / 单吊销 / 全吊销（命令去留由 A8 裁决，契约先锁定）", async () => {
    invokeMock.mockResolvedValueOnce([{ id: "d1", name: "phone", online: true }]);
    expect(await remoteDevices()).toEqual([{ id: "d1", name: "phone", online: true }]);
    expect(invokeMock).toHaveBeenCalledWith("remote_devices");
    await remoteRevokeDevice("d1");
    expect(invokeMock).toHaveBeenCalledWith("remote_revoke_device", { id: "d1" });
    await remoteRevokeAllDevices();
    expect(invokeMock).toHaveBeenCalledWith("remote_revoke_all_devices");
  });
});
