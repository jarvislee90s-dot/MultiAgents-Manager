import { beforeEach, beforeAll, describe, expect, it, vi } from "vitest";
import { fireEvent, screen, waitFor } from "@testing-library/react";
import { render } from "@testing-library/react";
import { VoiceGroupEditor } from "@/components/pet/manage/VoiceGroupEditor";
// tests/setup.ts 未初始化 i18n：coverage 文案带插值（{{groups}}），需真实 i18n 渲染
import i18n from "@/i18n";

beforeAll(async () => {
  await i18n.changeLanguage("zh");
});

const pickFiles = vi.fn();

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (...args: unknown[]) => pickFiles(...args),
}));

const rows = [
  { group: "general", name: "a", file: "voice/general/a.mp3", sizeBytes: 1000, durationMs: 3000 },
  { group: "general", name: "b", file: "voice/general/b.mp3", sizeBytes: 1000, durationMs: 25000 }, // too-long
  { group: "approval", name: "c", file: "voice/approval/c.mp3", sizeBytes: 1000, durationMs: null }, // 未探测
];

describe("VoiceGroupEditor", () => {
  beforeEach(() => {
    pickFiles.mockResolvedValue(["C:/x/new.mp3"]);
  });

  it("渲染分组、文件、问题徽标与组含义 tooltip（EP9）", () => {
    render(<VoiceGroupEditor rows={rows} onAdd={() => {}} onRemove={() => {}} />);
    expect(screen.getByTestId("voice-group-general")).toBeInTheDocument();
    expect(screen.getByTestId("voice-row-voice/general/b.mp3")).toHaveTextContent(/too-long|≥20s|时长 ≥20s/);
    expect(screen.getByTestId("voice-group-general")).toHaveAttribute("title"); // 组含义 tooltip
  });

  it("添加按钮走文件选择器并回调路径", async () => {
    const onAdd = vi.fn();
    render(<VoiceGroupEditor rows={rows} onAdd={onAdd} onRemove={() => {}} />);
    fireEvent.click(screen.getByTestId("voice-add-general"));
    expect(pickFiles).toHaveBeenCalledWith(expect.objectContaining({ multiple: true }));
    // onAdd 经 await open(...) 后回调，微任务时序需 waitFor
    await waitFor(() => expect(onAdd).toHaveBeenCalledWith("general", ["C:/x/new.mp3"]));
  });

  it("移除按钮回调相对路径；覆盖提示按合法文件计算", () => {
    const onRemove = vi.fn();
    render(<VoiceGroupEditor rows={rows} onAdd={() => {}} onRemove={onRemove} />);
    fireEvent.click(screen.getByTestId("voice-remove-voice/general/a.mp3"));
    expect(onRemove).toHaveBeenCalledWith("voice/general/a.mp3");
    // 仅 general 1 条合法 → 缺 approval/done/error
    expect(screen.getByTestId("voice-coverage")).toHaveTextContent(/approval|缺少分组/);
  });
});