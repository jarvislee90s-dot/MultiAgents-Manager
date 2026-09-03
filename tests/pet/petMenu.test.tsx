// tests/pet/petMenu.test.tsx — 右键菜单：开关 / 大小三档 / 动作绑定子页 / 隐藏 / 关于（spec §9 B/D14/D15）
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { PetMenu } from "@/components/pet/PetMenu";
import { loadConfig } from "@/components/pet/petConfig";
// tests/setup.ts 未初始化 i18n，组件的 zh 文案断言需显式引入并固定中文
import i18n from "@/i18n";

beforeAll(async () => {
  await i18n.changeLanguage("zh");
});

describe("PetMenu", () => {
  beforeEach(() => localStorage.clear());

  it("主菜单：三开关 + 大小 + 三动作绑定 + 隐藏/关于（spec §9 B1-B11/D14）", () => {
    const onPreview = vi.fn();
    render(<PetMenu onClose={() => {}} onPreview={onPreview} onHide={() => {}} voiceCapable subtitleCapable />);
    expect(screen.getByText("🔊 出声")).toBeTruthy();
    expect(screen.getByText("💬 语音字幕")).toBeTruthy();
    expect(screen.getByText("🧲 物理坠落")).toBeTruthy();
    expect(screen.getByText("📏 大小")).toBeTruthy();
    expect(screen.getByText("🖱️ 双击动作")).toBeTruthy();
    expect(screen.getByText("🔴 红灯动作")).toBeTruthy();
    expect(screen.getByText("🟢 绿灯动作")).toBeTruthy();
    expect(screen.queryByText(/failed/i)).toBeNull(); // error 场景不出现（D14）
    expect(screen.getByText("🦊 隐藏桌宠")).toBeTruthy();
  });

  it("点出声开关：muted 翻转写配置（开=有声，spec B1）", () => {
    render(<PetMenu onClose={() => {}} onPreview={() => {}} onHide={() => {}} voiceCapable subtitleCapable />);
    fireEvent.click(screen.getByText("🔊 出声"));
    expect(loadConfig().muted).toBe(true);
  });

  it("动作子页：进入即预览、选择即生效并回主菜单（spec B4-B7）", () => {
    const onPreview = vi.fn();
    render(<PetMenu onClose={() => {}} onPreview={onPreview} onHide={() => {}} voiceCapable subtitleCapable />);
    fireEvent.click(screen.getByText("🟢 绿灯动作"));
    expect(onPreview).toHaveBeenCalled(); // 进入子页预览当前选中
    fireEvent.click(screen.getByText("委屈"));
    expect(loadConfig().doneAction).toBe("failed");
    // 选择后回主菜单：动作行可见、子页"返回"行消失（原 brief 断言 getByText("← 返回")
    // 与实现"回主菜单"矛盾，主菜单无返回行，故改为主菜单特征断言）
    expect(screen.getByText("🟢 绿灯动作")).toBeTruthy();
    expect(screen.queryByText("← 返回")).toBeNull();
  });

  it("大小子页：三档选择写配置（spec D15）", () => {
    render(<PetMenu onClose={() => {}} onPreview={() => {}} onHide={() => {}} voiceCapable subtitleCapable />);
    fireEvent.click(screen.getByText("📏 大小"));
    fireEvent.click(screen.getByText("大"));
    expect(loadConfig().scale).toBe(1.25);
  });
});

describe("能力门控（spec §5.2）", () => {
  beforeEach(() => localStorage.clear());

  it("voiceCapable=false 时声音行置灰不可切换", async () => {
    localStorage.setItem("mam-pet-config", JSON.stringify({ muted: false }));
    render(
      <PetMenu
        onClose={() => {}}
        onPreview={() => {}}
        onHide={() => {}}
        voiceCapable={false}
        subtitleCapable={false}
      />
    );
    const rows = screen.getAllByTestId("pet-menu-row-sound");
    fireEvent.click(rows[0]);
    // 未切换：配置仍 muted=false
    expect(JSON.parse(localStorage.getItem("mam-pet-config")!).muted).toBe(false);
    expect(rows[0]).toHaveAttribute("title"); // tooltip 说明原因（EP9）
  });
  it("voiceCapable=true 时点击正常切换", () => {
    localStorage.setItem("mam-pet-config", JSON.stringify({ muted: false }));
    render(
      <PetMenu onClose={() => {}} onPreview={() => {}} onHide={() => {}} voiceCapable subtitleCapable />
    );
    fireEvent.click(screen.getAllByTestId("pet-menu-row-sound")[0]);
    expect(JSON.parse(localStorage.getItem("mam-pet-config")!).muted).toBe(true);
  });
});
