// tests/pet/petSwitchDialog.test.tsx — 切换对话框：mismatch 待决关闭结算 / 卡片徽标即时刷新 /
// v1 缩略图比例（issue #33-4/#33-5/#33-8）
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { PetSwitchDialog } from "@/components/pet/manage/PetSwitchDialog";
import { tauriInvokeMock } from "../msw/tauriMocks";
// tests/setup.ts 未初始化 i18n：徽标 title 与按钮文案需真实 i18n 渲染（固定英文）
import i18n from "@/i18n";

beforeAll(async () => {
  await i18n.changeLanguage("en");
});

const manifest = {
  id: "p1",
  displayName: "P",
  hasVoice: true,
  hasSubtitle: true,
  spriteVersionNumber: 1,
  spritesheetSizeBytes: 100,
  voices: [
    { group: "general", name: "greet", file: "voice/general/greet.mp3", sizeBytes: 10, durationMs: 3000 },
  ],
};
const pets = [
  { id: "p1", displayName: "P", spriteVersionNumber: 1, hasVoice: true, hasSubtitle: true, manifestExists: true, dir: "/x/p1" },
];

// ignore 场景：manifest 记录 greet 但磁盘没有（另有 done/other）→ mismatch（voice-missing + voice-extra）
function mockBackend(voiceFiles: { rel: string; size: number }[]) {
  tauriInvokeMock.mockImplementation((cmd: string) => {
    if (cmd === "pet_list_pets") return Promise.resolve(pets);
    if (cmd === "pet_scan")
      return Promise.resolve({
        id: "p1",
        dir: "/x/p1",
        spritesheet: { rel: "spritesheet.webp", exists: true, size: 100 }, // 大小一致 → 走稳态快路径，无需图集解码
        voiceFiles: voiceFiles.map((f) => ({ rel: f.rel, exists: true, size: f.size })),
      });
    if (cmd === "pet_read_manifest") return Promise.resolve(manifest);
    return Promise.resolve(undefined);
  });
}

describe("PetSwitchDialog（issue #33-4/#33-5/#33-8）", () => {
  beforeEach(() => {
    localStorage.clear();
    tauriInvokeMock.mockClear();
  });

  it("mismatch 待决时关闭对话框 → 按 cancel 结算，重开后 mismatch 清空且卡片可点（不悬挂 busy）", async () => {
    mockBackend([{ rel: "voice/done/other.mp3", size: 5 }]);
    const onOpenChange = vi.fn();
    const { rerender } = render(<PetSwitchDialog open onOpenChange={onOpenChange} />);
    fireEvent.click(await screen.findByTestId("pet-card-p1"));
    await screen.findByTestId("pet-switch-mismatch");
    // 旧实现：Dialog 关闭不 resolve mismatch promise → busy 恒 true、重开点按钮「补结算」
    rerender(<PetSwitchDialog open={false} onOpenChange={onOpenChange} />);
    rerender(<PetSwitchDialog open onOpenChange={onOpenChange} />);
    expect(await screen.findByTestId("pet-switch-list")).toBeInTheDocument(); // 回到卡片列表
    await waitFor(() => expect(screen.getByTestId("pet-card-p1")).toBeEnabled()); // busy 已复位
    expect(screen.queryByTestId("pet-switch-mismatch")).toBeNull();
  });

  it("ignore 降级激活后卡片能力徽标即时刷新（manifest 条目缺失 → 🔊 熄灭）", async () => {
    mockBackend([{ rel: "voice/done/other.mp3", size: 5 }]);
    render(<PetSwitchDialog open onOpenChange={() => {}} />);
    fireEvent.click(await screen.findByTestId("pet-card-p1"));
    fireEvent.click(
      await screen.findByRole("button", { name: "Ignore, run degraded from disk" })
    );
    await waitFor(() => expect(localStorage.getItem("mam-pet-voice-cap")).toBe("0"));
    // 卡片 🔊 徽标从点亮变熄灭（旧实现仍 text-primary）
    const card = screen.getByTestId("pet-card-p1");
    await waitFor(() => {
      expect(card.querySelector(".opacity-40")).not.toBeNull();
      expect(card.querySelector(".text-primary")).toBeNull();
    });
  });

  it("v1 卡片缩略图纵向比例 468（旧实现恒 572 压扁 v1）", async () => {
    mockBackend([]);
    render(<PetSwitchDialog open onOpenChange={() => {}} />);
    const card = await screen.findByTestId("pet-card-p1"); // pet_list_pets 异步加载
    const thumb = card.querySelector("div[style*='background-size']") as HTMLElement | null;
    expect(thumb?.style.backgroundSize).toBe("384px 468px");
  });
});
