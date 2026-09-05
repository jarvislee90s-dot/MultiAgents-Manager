import { beforeEach, beforeAll, describe, expect, it, vi } from "vitest";
import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { render } from "@testing-library/react";
import { PetManageDialog } from "@/components/pet/manage/PetManageDialog";
import { probeAudioDurationMs } from "@/components/pet/petRuntime";
import { repairManifest } from "@/components/pet/petActivation";
import { tauriInvokeMock } from "../msw/tauriMocks";
// tests/setup.ts 未初始化 i18n：徽标/时长文案需真实 i18n 渲染
import i18n from "@/i18n";

beforeAll(async () => {
  await i18n.changeLanguage("zh");
});

vi.mock("@/components/pet/petActivation", async (importOriginal) => {
  const orig = await importOriginal<typeof import("@/components/pet/petActivation")>();
  return {
    ...orig,
    buildManifestFromScan: vi.fn(),
    repairManifest: vi.fn().mockResolvedValue({ hasVoice: true, displayName: "P" }),
  };
});
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn().mockResolvedValue(["C:/a.mp3"]) }));
vi.mock("@tauri-apps/api/event", () => ({
  emit: vi.fn(async () => {}),
  listen: vi.fn(async () => () => {}),
}));
import { emit } from "@tauri-apps/api/event";
const emitMock = vi.mocked(emit);
vi.mock("@/components/pet/petRuntime", async (importOriginal) => {
  const orig = await importOriginal<typeof import("@/components/pet/petRuntime")>();
  return {
    ...orig,
    probeSheetRows: vi.fn().mockResolvedValue(9),
    probeAudioDurationMs: vi.fn().mockResolvedValue(3000),
  };
});

const pets = [
  {
    id: "starry-dew",
    displayName: "Starry Dew",
    spriteVersionNumber: 1,
    hasVoice: false,
    hasSubtitle: false,
    manifestExists: true,
    spritesheetExists: true,
    dir: "/x/starry-dew",
    source: "folder",
    description: "",
  },
];

describe("PetManageDialog", () => {
  beforeEach(() => {
    localStorage.clear();
    tauriInvokeMock.mockClear();
    tauriInvokeMock.mockImplementation((cmd: string) => {
      if (cmd === "pet_list_pets") return Promise.resolve(pets);
      if (cmd === "pet_scan")
        return Promise.resolve({
          id: "starry-dew",
          dir: "/x/starry-dew",
          spritesheet: { rel: "spritesheet.webp", exists: true, size: 100 },
          voiceFiles: [],
        });
      if (cmd === "pet_read_manifest")
        return Promise.resolve({
          id: "starry-dew",
          displayName: "Starry Dew",
          hasVoice: false,
          hasSubtitle: false,
          spriteVersionNumber: 1,
          spritesheetSizeBytes: 100,
          voices: [],
        });
      if (cmd === "pet_add_voice_files")
        return Promise.resolve([
          { group: "general", name: "x", file: "voice/general/x.mp3", sizeBytes: 1 },
        ]);
      return Promise.resolve(undefined);
    });
  });

  it("列表 → 选中进入面板并渲染字段", async () => {
    render(<PetManageDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("manage-pick-starry-dew"));
    expect(await screen.findByTestId("manage-panel")).toBeInTheDocument();
    expect(await screen.findByTestId("manage-rename-input")).toBeInTheDocument();
  });

  it("重命名激活中的宠物：经 foxbell 闪切，完成后指针回到新 id（EP5 修订/Bug3）", async () => {
    localStorage.setItem("mam-pet-active", "starry-dew");
    render(<PetManageDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("manage-pick-starry-dew"));
    fireEvent.change(await screen.findByTestId("manage-rename-input"), {
      target: { value: "dew" },
    });
    fireEvent.click(await screen.findByTestId("manage-rename-btn"));
    await waitFor(() =>
      expect(tauriInvokeMock.mock.calls.find((c) => c[0] === "pet_rename_pet")?.[1]).toEqual({
        oldId: "starry-dew",
        newId: "dew",
      })
    );
    // 闪切路径：rename 成功后自动切回（指针 = 新 id，事件已发出）
    await waitFor(() => expect(localStorage.getItem("mam-pet-active")).toBe("dew"));
    await waitFor(() => expect(emitMock).toHaveBeenCalledWith("pet-active-changed", {}));
  });

  it("重命名非激活宠物：指针不被动、无事件（Bug3）", async () => {
    localStorage.setItem("mam-pet-active", "foxbell");
    emitMock.mockClear();
    render(<PetManageDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("manage-pick-starry-dew"));
    fireEvent.change(await screen.findByTestId("manage-rename-input"), {
      target: { value: "dew" },
    });
    fireEvent.click(await screen.findByTestId("manage-rename-btn"));
    await waitFor(() =>
      expect(tauriInvokeMock.mock.calls.find((c) => c[0] === "pet_rename_pet")?.[1]).toEqual({
        oldId: "starry-dew",
        newId: "dew",
      })
    );
    expect(localStorage.getItem("mam-pet-active")).toBe("foxbell"); // 未激活：不切回、不广播
    expect(emitMock).not.toHaveBeenCalledWith("pet-active-changed", {});
  });

  it("保存激活宠物：重建 manifest（backup=true）后指针回到原 id 并广播（Bug3）", async () => {
    localStorage.setItem("mam-pet-active", "starry-dew");
    emitMock.mockClear();
    render(<PetManageDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("manage-pick-starry-dew"));
    fireEvent.click(await screen.findByTestId("manage-save"));
    await waitFor(() => {
      const call = tauriInvokeMock.mock.calls.find((c) => c[0] === "pet_update_manifest");
      expect(call?.[1]?.backup).toBe(true);
    });
    await waitFor(() => expect(localStorage.getItem("mam-pet-active")).toBe("starry-dew"));
    await waitFor(() => expect(emitMock).toHaveBeenCalledWith("pet-active-changed", {}));
  });

  it("保存非激活宠物：指针不被动、无事件（Bug3）", async () => {
    localStorage.setItem("mam-pet-active", "foxbell");
    emitMock.mockClear();
    render(<PetManageDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("manage-pick-starry-dew"));
    fireEvent.click(await screen.findByTestId("manage-save"));
    await waitFor(() => {
      const call = tauriInvokeMock.mock.calls.find((c) => c[0] === "pet_update_manifest");
      expect(call?.[1]?.backup).toBe(true);
    });
    expect(localStorage.getItem("mam-pet-active")).toBe("foxbell");
    expect(emitMock).not.toHaveBeenCalledWith("pet-active-changed", {});
  });

  it("删除：确认后 pet_delete_pet", async () => {
    render(<PetManageDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("manage-pick-starry-dew"));
    fireEvent.click(await screen.findByTestId("manage-delete"));
    fireEvent.click(await screen.findByTestId("manage-delete-confirm"));
    await waitFor(() =>
      expect(tauriInvokeMock.mock.calls.find((c) => c[0] === "pet_delete_pet")?.[1]?.id).toBe(
        "starry-dew"
      )
    );
  });

  it("先增删音频再保存：仍自动切回原宠物（EP5 修订，P1-4）", async () => {
    localStorage.setItem("mam-pet-active", "starry-dew");
    emitMock.mockClear();
    render(<PetManageDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("manage-pick-starry-dew"));
    // 添加音频触发直写保护：指针闪切回 foxbell（此后 doSave 若只读调用时点指针会恒判未激活）
    fireEvent.click(await screen.findByTestId("voice-add-general"));
    await waitFor(() => expect(localStorage.getItem("mam-pet-active")).toBe("foxbell"));
    fireEvent.click(screen.getByTestId("manage-save"));
    // 保存完成自动切回原宠物
    await waitFor(() => expect(localStorage.getItem("mam-pet-active")).toBe("starry-dew"));
    await waitFor(() => expect(emitMock).toHaveBeenCalledWith("pet-active-changed", {}));
  });

  it("描述可编辑并随保存写入 manifest（P1-7）", async () => {
    const repairMock = vi.mocked(repairManifest);
    repairMock.mockClear();
    render(<PetManageDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("manage-pick-starry-dew"));
    fireEvent.change(await screen.findByTestId("manage-desc-input"), {
      target: { value: "新的描述" },
    });
    fireEvent.click(screen.getByTestId("manage-save"));
    // 编辑后的 description 经 repairManifest 入参传入（repairManifest 展开透传，最终写入 manifest）
    await waitFor(() => {
      expect(repairMock).toHaveBeenCalledWith(
        expect.objectContaining({ description: "新的描述" }),
        expect.anything(),
        9
      );
    });
    const call = tauriInvokeMock.mock.calls.find((c) => c[0] === "pet_update_manifest");
    expect(call).toBeTruthy();
  });

  it("选中宠物：manifest 未缓存时长的行经共享 hook 探测后徽标从 no-duration 变为时长", async () => {
    // manifest 返回 voices（durationMs 缺失），磁盘 scan 同文件在 manifest 中（不产生 extra 重复）
    let resolveProbe: ((v: number) => void) | null = null;
    vi.mocked(probeAudioDurationMs).mockImplementation(
      () => new Promise<number>((res) => (resolveProbe = res))
    );
    tauriInvokeMock.mockImplementation((cmd: string) => {
      if (cmd === "pet_list_pets") return Promise.resolve(pets);
      if (cmd === "pet_scan")
        return Promise.resolve({
          id: "starry-dew",
          dir: "/x/starry-dew",
          spritesheet: { rel: "spritesheet.webp", exists: true, size: 100 },
          voiceFiles: [{ rel: "voice/general/greet.mp3", exists: true, size: 1000 }],
        });
      if (cmd === "pet_read_manifest")
        return Promise.resolve({
          id: "starry-dew",
          displayName: "Starry Dew",
          hasVoice: false,
          hasSubtitle: false,
          spriteVersionNumber: 1,
          spritesheetSizeBytes: 100,
          voices: [
            {
              group: "general",
              name: "greet",
              file: "voice/general/greet.mp3",
              sizeBytes: 1000,
              durationMs: null as unknown as number,
            },
          ],
        });
      return Promise.resolve(undefined);
    });

    render(<PetManageDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("manage-pick-starry-dew"));

    // 探测 pending：徽标为 no-duration
    const row = await screen.findByTestId("voice-row-voice/general/greet.mp3");
    expect(row).toHaveTextContent(/无法读取时长/);
    expect(probeAudioDurationMs).toHaveBeenCalled();

    // 探测 resolve（3s）：徽标消解，行内出现时长
    await act(async () => {
      resolveProbe?.(3000);
    });
    await waitFor(() => {
      expect(row).not.toHaveTextContent(/无法读取时长/);
      expect(row).toHaveTextContent(/3\.0s/);
    });
  });
});
