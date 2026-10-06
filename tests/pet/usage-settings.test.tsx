// tests/pet/usage-settings.test.tsx — 计划② Task 14：设置页「用量统计」分组（P7 的 8 项设置）验收面。
//
// 覆盖（9 条）：① 设置页接线可渲染（菜单项 → 分区 → 8 项全在场，且**不含** ① 的「采集状态 / 立即采集」
// 验收面）；② 开关改动即发 patch，并用**后端返回的合并对象**回写本地态；③ 数字输入过滤非数字；
// ④ 条数夹取上下界（99→7、0→1）；⑤ 范围 / 姿态是原生 select（3 档 / `poseKeysFor(11)` 的 11 项含
// `look`）；⑥ 保留期提示随值更新且夹取 1–3650；⑦ 规则校验与后端 `merge_patch` 同判据（合法 JSON
// 但形态不符同样提示）且**只提示不阻断**（patch 照发）；⑧ 评语占位符提示 + 保存失败原因可见
// （`usage-set-error`，不静默）；⑨ 换宠物 → 姿态非 `random` 时回退 `random` 并提示（已是则幂等）。
//
// **假后端**（不是桩）：`usage_get_settings` 读 `stored`、`usage_set_settings` 按后端 `merge_patch`
// 同语义合并回写 `stored` 并返回**完整 8 字段** —— 用例 2 靠它证明「回写的是返回的合并对象」而不是
// 「本地 patch 合并」的假绿（本地合并会把另一 WebView 已改的字段丢掉）。
//
// ⚠️ 本文件里**不得**出现轮询字段字面量（① 的源码锁按纯文本扫描「含该字面量的前端文件」，并要求
// 这类文件里不出现用量命令的 snake_case 字面量；扫描面只含 `src/`，但这里同样照此纪律写）；
// 命令名一律用 camelCase 包装名称呼。
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

// 本文件自持一套 invoke 实现（假后端 + 失败注入）；convertFileSrc 供 petRuntime 取用
vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  convertFileSrc: (p: string) => `asset://mock/${p}`,
}));
// WindowFrame / TitleBar（用例 1 渲染整个设置页）依赖 Tauri 窗口 API 与语言事件（照 petSettings.test.tsx）
vi.mock("@tauri-apps/api/webviewWindow", () => ({
  getCurrentWebviewWindow: () => ({
    isMaximized: vi.fn(async () => false),
    onResized: vi.fn(async () => () => {}),
    minimize: vi.fn(),
    toggleMaximize: vi.fn(),
    close: vi.fn(),
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));

// jsdom 未实现 matchMedia，ThemeProviders（WindowFrame 内置）需要它
window.matchMedia = ((query: string) => ({
  matches: false,
  media: query,
  onchange: null,
  addListener: vi.fn(),
  removeListener: vi.fn(),
  addEventListener: vi.fn(),
  removeEventListener: vi.fn(),
  dispatchEvent: vi.fn(),
})) as unknown as typeof window.matchMedia;

import i18n from "@/i18n";
import SettingsPage from "@/pages/settings";
import { UsageSection } from "@/components/settings/UsageSection";
import { saveActiveId } from "@/components/pet/petRuntime";
import { poseKeysFor } from "@/lib/usage/sheet";
import type { UsageSettings, UsageSettingsPatch } from "@/types/usage";

/** 与 Rust `UsageSettings::default()` 同值（spec §P7）：开 / 今日 / 3 条 / 90 天 / 10 分钟 / 空 / 空 / 随机 */
const DEFAULTS: UsageSettings = {
  enabled: true,
  miniBarRange: "today",
  miniBarToolRows: 3,
  detailRetentionDays: 90,
  collectIntervalMin: 10,
  providerMapRules: "",
  exportQuote: "",
  exportPose: "random",
};

/** 假后端当前落库值（用例可中途改写，模拟「另一个 WebView 改过设置」） */
let stored: UsageSettings;
/** 让下一次写入整包拒绝（形状与后端 `usage-settings-invalid` 一致，用完即清） */
let failNext: unknown;

function installBackend() {
  stored = { ...DEFAULTS };
  failNext = null;
  invokeMock.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    if (cmd === "usage_get_settings") return { ...stored };
    if (cmd === "usage_set_settings") {
      if (failNext) {
        const e = failNext;
        failNext = null;
        throw e; // 后端 `usage_set_settings` 整包拒绝：结构化的 { code, detail }
      }
      // 与 `merge_patch` 同语义：patch 未提的字段保持原值；返回合并后的**完整 8 字段**
      stored = { ...stored, ...((args?.patch ?? {}) as UsageSettingsPatch) };
      return { ...stored };
    }
    return null;
  });
}

/** 本次用例发过的全部 patch（顺序即调用顺序） */
const setPatches = (): UsageSettingsPatch[] =>
  invokeMock.mock.calls
    .filter(([cmd]) => cmd === "usage_set_settings")
    .map(([, args]) => (args as { patch: UsageSettingsPatch }).patch);

/** 渲染包一层 QueryClientProvider（设置查询走 `useUsageSettingsQuery`）；每用例独立 cache */
const renderWithClient = (ui: ReactNode) =>
  render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      {ui}
    </QueryClientProvider>
  );

const renderSection = () => renderWithClient(<UsageSection />);

const input = (testId: string) => screen.getByTestId(testId) as HTMLInputElement;
const select = (testId: string) => screen.getByTestId(testId) as HTMLSelectElement;

beforeAll(async () => {
  await i18n.changeLanguage("en");
});

beforeEach(() => {
  localStorage.clear();
  installBackend();
});

describe("设置页「用量统计」分组（计划② Task 14）", () => {
  it("1. 设置页接线：菜单进入分区，8 项设置全部在场且不掺 ① 的采集验收面", async () => {
    renderWithClient(<SettingsPage />);
    // ② 的四处改动（联合类型 / menuItems / 渲染分支 / lucide 导入）在**运行时**一起被这条用例走过：
    // 少任何一处，菜单点开就是空分区或直接编译不过
    fireEvent.click(await screen.findByRole("button", { name: "Usage statistics" }));

    expect(await screen.findByTestId("usage-enabled")).toBeTruthy();
    expect(screen.getByTestId("usage-minibar-range")).toBeTruthy();
    expect(screen.getByTestId("usage-minibar-tool-rows")).toBeTruthy();
    expect(screen.getByTestId("usage-detail-retention-days")).toBeTruthy();
    expect(screen.getByTestId("usage-collect-interval-min")).toBeTruthy();
    expect(screen.getByTestId("usage-provider-map-rules")).toBeTruthy();
    expect(screen.getByTestId("usage-export-quote")).toBeTruthy();
    expect(screen.getByTestId("usage-export-pose")).toBeTruthy();

    // 8 行标题（zh/en 同父同键的 8 个 label key）
    for (const label of [
      "Enable usage statistics",
      "Mini bar range",
      "Mini bar tool rows",
      "Detail retention (days)",
      "Fallback collect interval (minutes)",
      "Provider mapping rules (JSON)",
      "Share-image caption",
      "Share-image pose",
    ]) {
      expect(screen.getByText(label)).toBeTruthy();
    }

    // 与计划① 的验收面**不重复**：本分区里没有「采集状态 / 立即采集」（那是 UsageStatusSection 的面）
    expect(screen.queryByTestId("usage-status-collect")).toBeNull();
    expect(screen.queryByTestId("usage-status-collecting")).toBeNull();

    // 首帧值来自后端（不是前端硬编码默认值）
    expect(select("usage-minibar-range").value).toBe("today");
    expect(input("usage-minibar-tool-rows").value).toBe("3");
    expect(input("usage-detail-retention-days").value).toBe("90");
    expect(input("usage-collect-interval-min").value).toBe("10");
    expect(select("usage-export-pose").value).toBe("random");
  });

  it("2. 开关：改动即发 patch，并用后端返回的合并对象回写本地态", async () => {
    renderSection();
    const sw = await screen.findByTestId("usage-enabled");
    // 模拟另一个 WebView 已改过设置：落库值与本次初始读到的缓存不同 —— 只有「以返回的合并对象回写」
    // 才能把 7 显示出来；本地 `{...form, ...patch}` 合并会把这一改动丢掉（本条的判别力所在）
    stored = { ...stored, detailRetentionDays: 7 };

    fireEvent.click(sw);

    await waitFor(() => expect(setPatches()).toEqual([{ enabled: false }]));
    expect(sw.getAttribute("data-state")).toBe("unchecked");
    await waitFor(() => expect(input("usage-detail-retention-days").value).toBe("7"));
  });

  it("3. 数字输入：非数字被过滤掉，剩下的数字即时保存", async () => {
    renderSection();
    await screen.findByTestId("usage-detail-retention-days");
    const field = input("usage-detail-retention-days");

    fireEvent.change(field, { target: { value: "3a0b" } });

    // 过滤（不是 parseInt / Number：那会得到 3 或 NaN），落回输入框的是 30
    expect(field.value).toBe("30");
    await waitFor(() => expect(setPatches()).toEqual([{ detailRetentionDays: 30 }]));
  });

  it("4. 条数夹取：99 → 7、0 → 1（上下界都夹，且只发夹取后的值）", async () => {
    renderSection();
    await screen.findByTestId("usage-minibar-tool-rows");
    const field = input("usage-minibar-tool-rows");

    fireEvent.change(field, { target: { value: "99" } });
    expect(field.value).toBe("7");
    await waitFor(() => expect(setPatches()).toEqual([{ miniBarToolRows: 7 }]));

    fireEvent.change(field, { target: { value: "0" } });
    expect(field.value).toBe("1");
    await waitFor(() =>
      expect(setPatches()).toEqual([{ miniBarToolRows: 7 }, { miniBarToolRows: 1 }])
    );

    // 发出去的每个值都必须在后端白名单域内（merge_patch 越界是整包拒绝，不静默夹取）
    for (const patch of setPatches()) {
      expect(patch.miniBarToolRows).toBeGreaterThanOrEqual(1);
      expect(patch.miniBarToolRows).toBeLessThanOrEqual(7);
    }
  });

  it("5. 范围与姿态都是原生 select：范围 3 档、姿态 11 项（键序 = poseKeysFor(11)，含 look）", async () => {
    renderSection();
    await screen.findByTestId("usage-minibar-range");
    const range = select("usage-minibar-range");
    // 仓内没有 shadcn Select → 原生元素（不是 button+listbox 组合）
    expect(range.tagName).toBe("SELECT");
    expect(Array.from(range.options).map((o) => o.value)).toEqual(["today", "last5h", "last7d"]);
    expect(Array.from(range.options).map((o) => o.textContent)).toEqual([
      "Today",
      "Last 5 hours",
      "Last 7 days",
    ]);

    const pose = select("usage-export-pose");
    expect(pose.tagName).toBe("SELECT");
    // 键族唯一出处 = Task 12 的 `poseKeysFor`（不得自造第二份 POSE_KEYS）
    expect(Array.from(pose.options).map((o) => o.value)).toEqual(poseKeysFor(11));
    expect(pose.options).toHaveLength(11);
    // `look` 项不得删：9 行图集上没有那一行 → 文案里写明会回落待机
    const look = Array.from(pose.options).find((o) => o.value === "look");
    expect(look).toBeTruthy();
    expect(look?.textContent ?? "").toMatch(/falls back to idle/i);
  });

  it("6. 保留期提示：明示「N 天前无明细」，随值更新且夹取 1–3650", async () => {
    renderSection();
    const hint = await screen.findByTestId("usage-detail-retention-hint");
    expect(hint.textContent).toContain("90");
    expect(hint.textContent ?? "").toMatch(/not kept/i);

    fireEvent.change(input("usage-detail-retention-days"), { target: { value: "3650" } });
    await waitFor(() => expect(hint.textContent).toContain("3650"));
    expect(hint.textContent ?? "").not.toContain("90");

    // 上界夹取（3650 上限之外的输入不会被发出去）
    fireEvent.change(input("usage-detail-retention-days"), { target: { value: "99999" } });
    expect(input("usage-detail-retention-days").value).toBe("3650");
    await waitFor(() => expect(setPatches()).toHaveLength(2));
    expect(setPatches()[1]).toEqual({ detailRetentionDays: 3650 });
    // 发出去的每个值都在后端白名单域内（1–3650；越界那边是整包拒绝）
    for (const patch of setPatches()) {
      expect(patch.detailRetentionDays).toBeGreaterThanOrEqual(1);
      expect(patch.detailRetentionDays).toBeLessThanOrEqual(3650);
    }
  });

  it("7. 规则校验与后端 merge_patch 同判据：合法 JSON 但形态不符同样提示，且 patch 照发（不阻断）", async () => {
    renderSection();
    await screen.findByTestId("usage-provider-map-rules");
    const area = screen.getByTestId("usage-provider-map-rules") as HTMLTextAreaElement;
    // 空串 = 合法（清空规则，后端 `parse_provider_rules` 早退空表）
    expect(screen.queryByTestId("usage-provider-map-invalid")).toBeNull();

    // 合法 JSON、形态不符（rules 空表）→ 同样要提示
    fireEvent.change(area, { target: { value: '{"rules":[]}' } });
    expect(screen.getByTestId("usage-provider-map-invalid")).toBeTruthy();
    // 非法只提示不阻断：patch 照发（真正的裁决在后端）
    await waitFor(() => expect(setPatches()).toEqual([{ providerMapRules: '{"rules":[]}' }]));

    // 合法 JSON、缺 provider（serde 反序列化整篇失败 → 后端同判据为非法）
    fireEvent.change(area, { target: { value: '{"rules":[{"prefix":"deepseek-"}]}' } });
    expect(screen.getByTestId("usage-provider-map-invalid")).toBeTruthy();
    await waitFor(() => expect(setPatches()).toHaveLength(2));

    // 合法形态（非空 prefix + provider）→ 提示消失，patch 照发
    fireEvent.change(area, {
      target: { value: '{"rules":[{"prefix":"deepseek-","provider":"volcengine"}]}' },
    });
    await waitFor(() => expect(screen.queryByTestId("usage-provider-map-invalid")).toBeNull());
    expect(setPatches()[2]).toEqual({
      providerMapRules: '{"rules":[{"prefix":"deepseek-","provider":"volcengine"}]}',
    });
  });

  it("8. 评语占位符提示 + 保存失败原因可见（usage-set-error，不静默）", async () => {
    renderSection();
    await screen.findByTestId("usage-export-quote");
    const quote = input("usage-export-quote");
    // 占位符提示可用变量（单花括号是字面量，不是 i18next 插值）
    for (const placeholder of ["{range}", "{tokens}", "{hitPct}", "{models}"]) {
      expect(quote.placeholder).toContain(placeholder);
    }

    // 后端整包拒绝（形态不符 / 越界）→ 原因必须可见，不得静默吞掉
    failNext = { code: "usage-settings-invalid", detail: "浮窗分工具条数须在 1–7" };
    // 组件在失败时 console.error 留痕（与其它命令失败同惯例）→ 用例内静音，避免污染套件输出
    const errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      fireEvent.change(quote, { target: { value: "今日 {tokens} tokens" } });
      const box = await screen.findByTestId("usage-set-error");
      expect(box.textContent).toContain("Failed to save usage settings");
      // 原因走 usage.rpc.<code> 码表 + detail 通道，不打印原始错误对象
      expect(box.textContent).toContain("Invalid usage settings");
      expect(box.textContent).toContain("浮窗分工具条数须在 1–7");
      // 失败不谎报成功：输入框保留用户刚输入的值（后端没接受，错误就在旁边）
      expect(quote.value).toBe("今日 {tokens} tokens");
      expect(errSpy).toHaveBeenCalled(); // 控制台也留痕，不静默
    } finally {
      errSpy.mockRestore();
    }
  });

  it("9. 换宠物：姿态非 random 时回退 random 并提示；已是 random 时幂等（不重复提交）", async () => {
    renderSection();
    await screen.findByTestId("usage-export-pose");
    const pose = select("usage-export-pose");
    expect(pose.value).toBe("random");

    fireEvent.change(pose, { target: { value: "waving" } });
    await waitFor(() => expect(setPatches()).toEqual([{ exportPose: "waving" }]));
    expect(pose.value).toBe("waving");
    expect(screen.queryByTestId("usage-pose-reset")).toBeNull();

    // 真实切换路径：petRuntime.saveActiveId 写激活指针 + 派发同窗口事件
    // （跨窗口那一侧由 subscribeConfig 的 storage 分支覆盖，本用例走的是同一条订阅）
    await act(async () => {
      saveActiveId("starry-dew", false, "Starry Dew");
    });
    await waitFor(() =>
      expect(setPatches()).toEqual([{ exportPose: "waving" }, { exportPose: "random" }])
    );
    await waitFor(() => expect(pose.value).toBe("random"));
    const notice = await screen.findByTestId("usage-pose-reset");
    expect(notice.textContent ?? "").toMatch(/fell back to Random/i);

    // 已回退成 random 之后再换宠物：不重复提交、也不新增提示（幂等）
    await act(async () => {
      saveActiveId("another-pet", false, "Another");
    });
    expect(setPatches()).toHaveLength(2);
  });
});
