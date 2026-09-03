import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { PetStartupGuard } from "@/components/pet/PetStartupGuard";
import { tauriInvokeMock } from "../msw/tauriMocks";

// pet-activation 的 repair 复用（mock 为可直接断言的桩）
vi.mock("@/components/pet/petActivation", async (importOriginal) => {
  const orig = await importOriginal<typeof import("@/components/pet/petActivation")>();
  return { ...orig, repairManifest: vi.fn().mockResolvedValue({ hasVoice: false, displayName: "P" }) };
});

const manifest = {
  id: "p1", displayName: "P", hasVoice: false, hasSubtitle: false,
  spriteVersionNumber: 2, spritesheetSizeBytes: 100, voices: [],
};
const scanOk = {
  id: "p1", dir: "/x/p1",
  spritesheet: { rel: "spritesheet.webp", exists: true, size: 100 },
  voiceFiles: [{ rel: "voice/done/new.mp3", exists: true, size: 5 }], // extra → issue
};

describe("PetStartupGuard（EP2 启动弹窗）", () => {
  beforeEach(() => {
    localStorage.clear();
    tauriInvokeMock.mockClear();
  });

  it("foxbell 激活时不弹窗", async () => {
    localStorage.setItem("mam-pet-active", "foxbell");
    const { container } = render(<PetStartupGuard />);
    await waitFor(() => expect(tauriInvokeMock).not.toHaveBeenCalled());
    expect(container.querySelector("[data-testid='pet-startup-dialog']")).toBeNull();
  });

  it("素材不一致 → 弹窗三选；点更新 → pet_update_manifest(backup=true)", async () => {
    localStorage.setItem("mam-pet-active", "p1");
    tauriInvokeMock.mockImplementation((cmd: string) => {
      if (cmd === "pet_scan") return Promise.resolve(scanOk);
      if (cmd === "pet_read_manifest") return Promise.resolve(manifest);
      return Promise.resolve(undefined);
    });
    render(<PetStartupGuard />);
    const dlg = await screen.findByTestId("pet-startup-dialog");
    expect(dlg).toBeInTheDocument();
    fireEvent.click(await screen.findByTestId("pet-startup-update"));
    await waitFor(() => {
      const call = tauriInvokeMock.mock.calls.find((c) => c[0] === "pet_update_manifest");
      expect(call?.[1]?.backup).toBe(true);
    });
  });

  it("图集缺失 → 致命弹窗（无更新按钮，只有切回/关闭）", async () => {
    localStorage.setItem("mam-pet-active", "p1");
    tauriInvokeMock.mockImplementation((cmd: string) => {
      if (cmd === "pet_scan")
        return Promise.resolve({ ...scanOk, spritesheet: { rel: "spritesheet.webp", exists: false, size: 0 } });
      if (cmd === "pet_read_manifest") return Promise.resolve(manifest);
      return Promise.resolve(undefined);
    });
    render(<PetStartupGuard />);
    await screen.findByTestId("pet-startup-dialog");
    expect(screen.queryByTestId("pet-startup-update")).toBeNull();
    fireEvent.click(screen.getByTestId("pet-startup-foxbell"));
    await waitFor(() => expect(localStorage.getItem("mam-pet-active")).toBe("foxbell"));
  });
});