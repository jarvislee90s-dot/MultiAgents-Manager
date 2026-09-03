import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, screen, waitFor } from "@testing-library/react";
import { render } from "@testing-library/react";
import { PetImportDialog } from "@/components/pet/manage/PetImportDialog";
import { tauriInvokeMock } from "../msw/tauriMocks";

const pick = vi.fn();

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: (...a: unknown[]) => pick(...a) }));
vi.mock("@/components/pet/petRuntime", async (importOriginal) => {
  const orig = await importOriginal<typeof import("@/components/pet/petRuntime")>();
  return { ...orig, probeSheetRows: vi.fn().mockResolvedValue(9), probeAudioDurationMs: vi.fn().mockResolvedValue(3000) };
});

const staged = {
  stagingId: "s1",
  dir: "/home/u/.mam/pets/.import-staging/s1",
  suggestedName: "starry-dew",
  suggestedDisplayName: "Starry Dew",
  spriteVersionNumber: 0,
  spritesheetSize: 1652314,
  voiceFiles: [],
};

describe("PetImportDialog", () => {
  beforeEach(() => {
    tauriInvokeMock.mockClear();
    tauriInvokeMock.mockImplementation((cmd: string) => {
      if (cmd === "pet_stage_from_folder") return Promise.resolve(staged);
      if (cmd === "pet_finalize_import") return Promise.resolve({ id: staged.suggestedName, displayName: staged.suggestedDisplayName });
      return Promise.resolve(undefined);
    });
    pick.mockResolvedValue("C:/pets/starry-dew");
  });

  it("本地文件夹来源 → 暂存 → 配置页显示预览与名称 → 完成导入", async () => {
    render(<PetImportDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("import-tab-local"));
    fireEvent.click(await screen.findByTestId("import-pick-folder"));
    await screen.findByTestId("import-config");
    expect(await screen.findByDisplayValue("starry-dew")).toBeInTheDocument();
    expect(await screen.findByTestId("import-sheet-badge")).toHaveTextContent("v1"); // probe 桩返回 9
    fireEvent.click(await screen.findByTestId("import-execute"));
    await waitFor(() => {
      const call = tauriInvokeMock.mock.calls.find((c) => c[0] === "pet_finalize_import");
      expect(call?.[1]?.name).toBe("starry-dew");
      expect(call?.[1]?.manifest.spriteVersionNumber).toBe(1);
    });
    expect(await screen.findByTestId("import-done")).toBeInTheDocument();
  });

  it("petdex 渠道：输入链接 → pet_stage_from_petdex", async () => {
    tauriInvokeMock.mockImplementation((cmd: string) => {
      if (cmd === "pet_stage_from_petdex") return Promise.resolve(staged);
      if (cmd === "pet_finalize_import") return Promise.resolve({ id: "x" });
      return Promise.resolve(undefined);
    });
    render(<PetImportDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("import-tab-petdex"));
    fireEvent.change(await screen.findByTestId("import-petdex-url"), {
      target: { value: "https://petdex.dev/pets/capvolt" },
    });
    fireEvent.click(await screen.findByTestId("import-petdex-download"));
    expect(await screen.findByTestId("import-config")).toBeInTheDocument();
    expect(tauriInvokeMock.mock.calls.find((c) => c[0] === "pet_stage_from_petdex")?.[1]?.url).toBe(
      "https://petdex.dev/pets/capvolt"
    );
  });

  it("配置页关闭对话框 → pet_cancel_import 清理", async () => {
    const onOpenChange = vi.fn();
    render(<PetImportDialog open onOpenChange={onOpenChange} />);
    fireEvent.click(await screen.findByTestId("import-tab-local"));
    fireEvent.click(await screen.findByTestId("import-pick-folder"));
    await screen.findByTestId("import-config");
    fireEvent.click(await screen.findByTestId("import-cancel"));
    await waitFor(() =>
      expect(tauriInvokeMock.mock.calls.find((c) => c[0] === "pet_cancel_import")?.[1]?.stagingId).toBe("s1")
    );
  });
});