// tests/session/jumpFallbackHint.test.ts — review M3：CLI 会话 TTY 聚焦失败走 APP 级
// 保底（via=app-fallback）时给出一次性 UX 提示；APP 会话与 TTY 直达不提示
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock, toastInfoMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  toastInfoMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("sonner", () => ({ toast: { info: toastInfoMock, error: vi.fn(), success: vi.fn() } }));
// useSessionJump 现依赖 useTranslation；测试走默认 en（fallbackLng）
// tests/setup.ts 未初始化 i18n，显式引入（fallbackLng=en，与断言文案一致）
import i18n from "@/i18n";
import { useSessionJump } from "@/hooks/useSessionJump";

void i18n;

const target = {
  pid: 4242,
  id: "s-cli",
  agentType: "codex",
  projectName: "Demo",
  form: "cli" as const,
};

beforeEach(() => {
  invokeMock.mockReset();
  toastInfoMock.mockReset();
});

describe("CLI 兜底跳转 UX 提示（review M3）", () => {
  it("via=app-fallback + CLI 会话 → toast.info 提示宿主 APP 已激活", async () => {
    invokeMock.mockResolvedValue({ type: "focused", via: "app-fallback" });
    const { result } = renderHook(() => useSessionJump());
    await waitFor(async () => {
      await result.current.focus(target);
    });
    expect(toastInfoMock).toHaveBeenCalledWith(
      "CLI terminal not located; host app brought to front"
    );
  });

  it("TTY 直达（via=tty）不提示", async () => {
    invokeMock.mockResolvedValue({ type: "focused", via: "tty" });
    const { result } = renderHook(() => useSessionJump());
    await result.current.focus(target);
    expect(toastInfoMock).not.toHaveBeenCalled();
  });

  it("APP 会话走保底激活（预期行为）不提示", async () => {
    invokeMock.mockResolvedValue({ type: "focused", via: "app-fallback" });
    const { result } = renderHook(() => useSessionJump());
    await result.current.focus({ ...target, form: "app" });
    expect(toastInfoMock).not.toHaveBeenCalled();
  });
});
