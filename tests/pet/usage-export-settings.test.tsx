// 分享图评语编辑器（2026-10-07 用户裁决 B1：评语从「设置 → 用量统计」搬到**导出所在的地方**）。
//
// 这条用例守三件事，都是「搬了家但机制没跟上」会真出的问题：
//  ① **入口在看板的导出条上**（用户原话：会在这个导出的地方找），且与另三个看板口径按钮**同纪律**
//     （记录页签下在位但禁用，不是凭空消失）；
//  ② **读是惰性的**（挂载不发 IPC）—— 导出条住在吸顶头里，每次渲染都平白读一次设置既浪费、
//     又会打乱导出用例按调用序编排的一次性 mock；
//  ③ 读写走**与设置页同一条** `usage_get_settings` / `usage_set_settings`（不自立第二套），
//     失败按同一张码表可见、不静默。
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "@/i18n";
import { UsageExportSettings } from "@/components/usage/UsageExportSettings";
import { poseKeysFor } from "@/lib/usage/sheet";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

/** 记账：按命令名收集调用（断言「读了什么、写了什么、写了几次」） */
const callsOf = (cmd: string) => invokeMock.mock.calls.filter((c) => c[0] === cmd);

const SETTINGS = {
  enabled: false,
  minibarRange: "today",
  minibarToolRows: 3,
  detailRetentionDays: 90,
  collectIntervalMin: 10,
  providerMapRules: "",
  exportQuote: "旧评语 {tokens}",
  exportPose: "random",
};

// **裸渲染**（与生产一致）：导出条由 `UsageExportActions` 直接渲染，外面没有 QueryClientProvider。
// 本组件若哪天又需要 provider，这几条用例会立刻红 —— 那正是我们要的信号。
const renderEditor = () => render(<UsageExportSettings />);

describe("UsageExportSettings（B1：评语搬到导出处）", () => {
  // i18n 与既有导出用例同款：`@/i18n` 副作用导入 + 固定语言，否则 `t()` 只会回显 key
  beforeAll(async () => {
    await i18n.changeLanguage("en");
  });

  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "usage_get_settings") return SETTINGS;
      if (cmd === "usage_set_settings") return SETTINGS;
      return null;
    });
  });

  it("1. **挂载不发 IPC**（读是惰性的）——只有点「评语」才读一次设置", async () => {
    renderEditor();
    // 给微任务/渲染一个机会：此处若发过 IPC，下面这条就会红（含 useUsageSettingsQuery 的回归）
    await Promise.resolve();
    expect(callsOf("usage_get_settings")).toHaveLength(0);

    fireEvent.click(screen.getByTestId("usage-export-settings-open"));
    await waitFor(() => expect(callsOf("usage_get_settings")).toHaveLength(1));
    expect(screen.getByTestId("usage-export-quote-input")).toHaveValue("旧评语 {tokens}");
  });

  it("2. 占位符提示可用变量；保存走**同一条** usage_set_settings（只发 patch，不发整包）", async () => {
    renderEditor();
    fireEvent.click(screen.getByTestId("usage-export-settings-open"));
    const input = (await screen.findByTestId("usage-export-quote-input")) as HTMLInputElement;

    // 单花括号是**字面量**（占位符提示），不是 i18next 插值 —— 四个变量都要在
    for (const v of ["{range}", "{tokens}", "{hitPct}", "{models}"]) {
      expect(input.placeholder).toContain(v);
    }

    fireEvent.change(input, { target: { value: "今天写了 {tokens}" } });
    fireEvent.click(screen.getByTestId("usage-export-settings-save"));

    await waitFor(() => expect(callsOf("usage_set_settings")).toHaveLength(1));
    // 姿态（本轮从设置页搬来）与评语**一次 patch 提交**：分两次发会让「保存成功」变成
    // 「一半成功」✗ —— 故这里的期望值必须同时含两项
    expect(callsOf("usage_set_settings")[0][1]).toEqual({
      patch: { exportQuote: "今天写了 {tokens}", exportPose: "random" },
    });
    // 成功后对话框关闭 + 行内出「已保存」
    await waitFor(() => expect(screen.queryByTestId("usage-export-settings-dialog")).toBeNull());
    expect(screen.getByTestId("usage-export-settings-note")).toBeTruthy();
  });

  it("2b. 弹层里**姿态**选择器在场：11 项、键序 = poseKeysFor(11)、含 look（从设置页整体搬来）", async () => {
    renderEditor();
    fireEvent.click(screen.getByTestId("usage-export-settings-open"));
    const pose = (await screen.findByTestId("usage-export-pose")) as HTMLSelectElement;
    expect(pose.tagName).toBe("SELECT");
    expect(Array.from(pose.options).map((o) => o.value)).toEqual(poseKeysFor(11));
    expect(pose.options).toHaveLength(11);
    // `look` 项不得删：9 行图集上没有那一行 → 文案里写明会回落待机
    const look = Array.from(pose.options).find((o) => o.value === "look");
    expect(look?.textContent ?? "").toMatch(/falls back to idle/i);
    // 标签必须写明「是谁的姿态」（用户反馈「不太理解是什么的姿态」）
    expect(screen.getByText(/token/i)).toBeTruthy();
  });

  it("3. 失败：对话框**不关**、原因可见（同一条 usage.rpc 码表）、不谎报成功", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "usage_get_settings") return SETTINGS;
      if (cmd === "usage_set_settings") {
        throw { code: "usage-settings-invalid", detail: "评语过长" };
      }
      return null;
    });
    const errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      renderEditor();
      fireEvent.click(screen.getByTestId("usage-export-settings-open"));
      const input = await screen.findByTestId("usage-export-quote-input");
      fireEvent.change(input, { target: { value: "x" } });
      fireEvent.click(screen.getByTestId("usage-export-settings-save"));

      const box = await screen.findByTestId("usage-export-settings-error");
      // 码表把 usage-settings-invalid 译成人话，并把 detail 带出来（不打印原始错误对象）
      expect(box.textContent).toContain("评语过长");
      expect(errSpy).toHaveBeenCalled();
      // 失败不谎报：对话框还在、没有「已保存」
      expect(screen.getByTestId("usage-export-settings-dialog")).toBeTruthy();
      expect(screen.queryByTestId("usage-export-settings-note")).toBeNull();
    } finally {
      errSpy.mockRestore();
    }
  });
});
