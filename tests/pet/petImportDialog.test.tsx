import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, screen, waitFor } from "@testing-library/react";
import { render } from "@testing-library/react";
import { PetImportDialog } from "@/components/pet/manage/PetImportDialog";
import { tauriInvokeMock } from "../msw/tauriMocks";

const pick = vi.fn();
const openUrlMock = vi.fn();

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: (...a: unknown[]) => pick(...a) }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: (...a: unknown[]) => openUrlMock(...a) }));
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
    openUrlMock.mockClear();
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

  it("petdex 渠道：浏览按钮跳转 petdex.dev 首页（不再带 /collections）", async () => {
    render(<PetImportDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("import-tab-petdex"));
    fireEvent.click(await screen.findByTestId("import-petdex-browse"));
    expect(openUrlMock).toHaveBeenCalledWith("https://petdex.dev");
  });

  it("petdex 渠道：下载中显示 spinner 与“下载中…”文案，完成后恢复", async () => {
    let resolveStage: ((v: typeof staged) => void) | null = null;
    tauriInvokeMock.mockImplementation((cmd: string) => {
      if (cmd === "pet_stage_from_petdex")
        return new Promise((res) => (resolveStage = res as (v: typeof staged) => void));
      return Promise.resolve(undefined);
    });
    render(<PetImportDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("import-tab-petdex"));
    fireEvent.change(await screen.findByTestId("import-petdex-url"), {
      target: { value: "https://petdex.dev/pets/capvolt" },
    });
    fireEvent.click(await screen.findByTestId("import-petdex-download"));
    // busy：按钮禁用、文案切换为“下载中…”、出现 spinner（测试环境 i18n 未初始化，渲染键名）
    const btn = await screen.findByTestId("import-petdex-download");
    expect(btn).toBeDisabled();
    expect(btn).toHaveTextContent("pet.import.petdexDownloading");
    expect(btn.querySelector(".animate-spin")).not.toBeNull();
    // 完成：进入配置页（busy 结束，来源页按钮随之卸载）
    const { act } = await import("@testing-library/react");
    await act(async () => resolveStage?.(staged));
    await screen.findByTestId("import-config");
    expect(screen.queryByTestId("import-petdex-download")).toBeNull();
  });

  it("petdex 渠道：下载失败 → toast 报错、按钮恢复原文案", async () => {
    const { toast } = await import("sonner");
    const toastError = vi.spyOn(toast, "error").mockImplementation(() => "");
    tauriInvokeMock.mockImplementation((cmd: string) => {
      if (cmd === "pet_stage_from_petdex") return Promise.reject(new Error("boom"));
      return Promise.resolve(undefined);
    });
    render(<PetImportDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("import-tab-petdex"));
    fireEvent.change(await screen.findByTestId("import-petdex-url"), {
      target: { value: "https://petdex.dev/pets/capvolt" },
    });
    fireEvent.click(await screen.findByTestId("import-petdex-download"));
    await waitFor(() => expect(toastError).toHaveBeenCalled());
    const btn = screen.getByTestId("import-petdex-download");
    expect(btn).not.toBeDisabled();
    expect(btn).toHaveTextContent("pet.import.petdexDownload");
    expect(btn.querySelector(".animate-spin")).toBeNull();
    toastError.mockRestore();
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
describe("PetImportDialog 探测竞态与校验（issue #33-3/#33-5/#33-7）", () => {
  beforeEach(() => {
    tauriInvokeMock.mockClear();
    tauriInvokeMock.mockImplementation((cmd: string) => {
      if (cmd === "pet_stage_from_folder") return Promise.resolve(staged);
      if (cmd === "pet_finalize_import") return Promise.resolve({ id: staged.suggestedName });
      return Promise.resolve(undefined);
    });
    pick.mockResolvedValue("C:/pets/starry-dew");
  });

  it("跨次暂存探测竞态：旧探测后到不得覆盖新暂存的行数（代数闸门）", async () => {
    const { probeSheetRows } = await import("@/components/pet/petRuntime");
    let resA: ((v: number) => void) | null = null;
    const pendingA = new Promise<number>((res) => (resA = res));
    vi.mocked(probeSheetRows)
      .mockImplementationOnce(() => pendingA) // A 的探测挂起
      .mockResolvedValueOnce(9); // B 的探测先回
    const { act } = await import("@testing-library/react");
    render(<PetImportDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("import-tab-local"));
    fireEvent.click(await screen.findByTestId("import-pick-folder"));
    await screen.findByTestId("import-config");
    // 取消 A → 回来源页 → 暂存 B
    fireEvent.click(await screen.findByTestId("import-cancel"));
    await waitFor(() => expect(screen.queryByTestId("import-config")).toBeNull());
    fireEvent.click(await screen.findByTestId("import-pick-folder"));
    await screen.findByTestId("import-config");
    expect(await screen.findByTestId("import-sheet-badge")).toHaveTextContent("v1"); // B = 9 行
    // A 的过期探测后到 → 不得覆盖 B 的 9
    await act(async () => resA?.(11));
    expect(screen.getByTestId("import-sheet-badge")).toHaveTextContent("v1");
  });

  it("预览比例随探测行数：v1 → 936、v2 → 1144（不再恒用 v2 压扁 v1）", async () => {
    const { probeSheetRows } = await import("@/components/pet/petRuntime");
    const mocked = vi.mocked(probeSheetRows);
    mocked.mockResolvedValue(9);
    render(<PetImportDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("import-tab-local"));
    fireEvent.click(await screen.findByTestId("import-pick-folder"));
    await screen.findByTestId("import-config");
    await waitFor(() =>
      expect(screen.getByTestId("import-preview").style.backgroundSize).toBe("768px 936px")
    );
    // 重暂存探测为 v2 → 1144
    mocked.mockResolvedValue(11);
    fireEvent.click(await screen.findByTestId("import-cancel"));
    await waitFor(() => expect(screen.queryByTestId("import-config")).toBeNull());
    fireEvent.click(await screen.findByTestId("import-pick-folder"));
    await screen.findByTestId("import-config");
    await waitFor(() =>
      expect(screen.getByTestId("import-preview").style.backgroundSize).toBe("768px 1144px")
    );
  });

  it("重名实时校验：输入已导入 id → 提示并禁用执行（不再等 finalize 才报错）", async () => {
    tauriInvokeMock.mockImplementation((cmd: string) => {
      if (cmd === "pet_list_pets")
        return Promise.resolve([{ id: "starry-dew", displayName: "Starry Dew" }]);
      if (cmd === "pet_stage_from_folder") return Promise.resolve(staged);
      return Promise.resolve(undefined);
    });
    render(<PetImportDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("import-tab-local"));
    fireEvent.click(await screen.findByTestId("import-pick-folder"));
    await screen.findByTestId("import-config");
    // suggestedName 即已导入 id → 进入配置页立刻提示（测试环境 i18n 未初始化，渲染键名）
    expect(await screen.findByTestId("import-name-problem")).toHaveTextContent("pet.import.nameDup");
    expect(screen.getByTestId("import-execute")).toBeDisabled();
  });
});
