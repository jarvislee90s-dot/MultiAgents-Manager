import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, screen, waitFor } from "@testing-library/react";
import { render } from "@testing-library/react";
import { PetManageDialog } from "@/components/pet/manage/PetManageDialog";
import { tauriInvokeMock } from "../msw/tauriMocks";

vi.mock("@/components/pet/petActivation", async (importOriginal) => {
  const orig = await importOriginal<typeof import("@/components/pet/petActivation")>();
  return { ...orig, buildManifestFromScan: vi.fn(), repairManifest: vi.fn().mockResolvedValue({ hasVoice: true, displayName: "P" }) };
});
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn().mockResolvedValue(["C:/a.mp3"]) }));
vi.mock("@/components/pet/petRuntime", async (importOriginal) => {
  const orig = await importOriginal<typeof import("@/components/pet/petRuntime")>();
  return { ...orig, probeSheetRows: vi.fn().mockResolvedValue(9), probeAudioDurationMs: vi.fn().mockResolvedValue(3000) };
});

const pets = [
  { id: "starry-dew", displayName: "Starry Dew", spriteVersionNumber: 1, hasVoice: false, hasSubtitle: false, manifestExists: true, spritesheetExists: true, dir: "/x/starry-dew", source: "folder", description: "" },
];

describe("PetManageDialog", () => {
  beforeEach(() => {
    localStorage.clear();
    tauriInvokeMock.mockClear();
    tauriInvokeMock.mockImplementation((cmd: string) => {
      if (cmd === "pet_list_pets") return Promise.resolve(pets);
      if (cmd === "pet_scan")
        return Promise.resolve({
          id: "starry-dew", dir: "/x/starry-dew",
          spritesheet: { rel: "spritesheet.webp", exists: true, size: 100 },
          voiceFiles: [],
        });
      if (cmd === "pet_read_manifest")
        return Promise.resolve({
          id: "starry-dew", displayName: "Starry Dew", hasVoice: false, hasSubtitle: false,
          spriteVersionNumber: 1, spritesheetSizeBytes: 100, voices: [],
        });
      return Promise.resolve(undefined);
    });
  });

  it("列表 → 选中进入面板并渲染字段", async () => {
    render(<PetManageDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("manage-pick-starry-dew"));
    expect(await screen.findByTestId("manage-panel")).toBeInTheDocument();
    expect(await screen.findByTestId("manage-rename-input")).toBeInTheDocument();
  });

  it("重命名激活中的宠物：先切回 foxbell 再 pet_rename_pet（EP5）", async () => {
    localStorage.setItem("mam-pet-active", "starry-dew");
    render(<PetManageDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("manage-pick-starry-dew"));
    fireEvent.change(await screen.findByTestId("manage-rename-input"), { target: { value: "dew" } });
    fireEvent.click(await screen.findByTestId("manage-rename-btn"));
    await waitFor(() =>
      expect(tauriInvokeMock.mock.calls.find((c) => c[0] === "pet_rename_pet")?.[1]).toEqual({
        oldId: "starry-dew",
        newId: "dew",
      })
    );
    expect(localStorage.getItem("mam-pet-active")).toBe("foxbell"); // 已先切回
  });

  it("保存：重建 manifest 并 pet_update_manifest(backup=true)", async () => {
    render(<PetManageDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("manage-pick-starry-dew"));
    fireEvent.click(await screen.findByTestId("manage-save"));
    await waitFor(() => {
      const call = tauriInvokeMock.mock.calls.find((c) => c[0] === "pet_update_manifest");
      expect(call?.[1]?.backup).toBe(true);
    });
  });

  it("删除：确认后 pet_delete_pet", async () => {
    render(<PetManageDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("manage-pick-starry-dew"));
    fireEvent.click(await screen.findByTestId("manage-delete"));
    fireEvent.click(await screen.findByTestId("manage-delete-confirm"));
    await waitFor(() =>
      expect(tauriInvokeMock.mock.calls.find((c) => c[0] === "pet_delete_pet")?.[1]?.id).toBe("starry-dew")
    );
  });
});