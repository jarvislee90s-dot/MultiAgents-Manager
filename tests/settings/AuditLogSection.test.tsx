// tests/settings/AuditLogSection.test.tsx — M7 W5：设置页「注入审计」桌面查看入口。
// 覆盖：挂载即 invoke inject_list_audit 渲染三条（设备名/动作原样小写/摘要/会话/短时间）/
// 空载荷 → 「暂无记录」空态且无列表行 / 刷新按钮触发再次 invoke（调用计数）/
// i18n zh-en 无缺键（settings.audit 子树，对齐 remoteSection.test.tsx 的子树版扫描）。
// mock 模式沿用 remoteSection.test.tsx（vi.hoisted + vi.mock，自带 invoke/event）。
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import path from "node:path";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
// useAppTranslation 内部 listen("@tauri-apps/api/event") 在 jsdom 无 Tauri 内核，须 mock
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));

// tests/setup.ts 未初始化 i18n，显式引入并按默认英文断言（jsdom navigator.language=en）
import i18n from "@/i18n";
import { AuditLogSection } from "@/components/settings/AuditLogSection";

void i18n;

// Rust AuditRow（serde camelCase，不含 device_id）同构样例，最新在前
const rowsOf = () => [
  {
    ts: 1758132000000,
    deviceName: "JARVIS 的 iPhone",
    agentType: "claude",
    sessionId: "sess-abc-1",
    channel: "tmux",
    action: "send",
    // 丁T3 裁2：签名后置（真机形态 `{正文} [mobile 设备名]`）
    summary: "修复登录页空指针 [mobile JARVIS 的 iPhone]",
    result: "ok",
  },
  {
    ts: 1758128400000,
    deviceName: "iPad",
    agentType: "codex",
    sessionId: "sess-def-2",
    channel: "tmux",
    action: "queue",
    summary: "跑一遍回归测试 [mobile iPad]",
    result: "ok",
  },
  {
    ts: 1758124800000,
    deviceName: "Desktop-A",
    agentType: "claude",
    sessionId: "sess-ghi-3",
    channel: "tmux",
    // 丁T3：斜杠命令裸注入——摘要即命令原文（终端不留痕，溯源靠本表 action+设备名）
    action: "slash",
    summary: "/permissions",
    result: "ok",
  },
];

// inject_list_audit 调用次数（不关心附带参数形态，只数命令名）
const auditCalls = () => invokeMock.mock.calls.filter((c) => c[0] === "inject_list_audit").length;

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === "inject_list_audit") return { items: rowsOf() };
    return null;
  });
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("AuditLogSection 渲染（M7 W5）", () => {
  it("挂载即 invoke inject_list_audit，列表出现设备名/动作/摘要（动作原样小写不翻译）", async () => {
    render(<AuditLogSection />);
    expect(await screen.findByText("JARVIS 的 iPhone")).toBeTruthy();
    expect(invokeMock).toHaveBeenCalledWith("inject_list_audit");
    expect(screen.getByText("iPad")).toBeTruthy();
    expect(screen.getByText("Desktop-A")).toBeTruthy();
    // 动作列：词表原样小写展示
    expect(screen.getByText("send")).toBeTruthy();
    expect(screen.getByText("queue")).toBeTruthy();
    expect(screen.getByText("slash")).toBeTruthy();
    // 摘要列：后端已截断的 summary 原文
    expect(screen.getByText("修复登录页空指针 [mobile JARVIS 的 iPhone]")).toBeTruthy();
    expect(screen.getByText("跑一遍回归测试 [mobile iPad]")).toBeTruthy();
    expect(screen.getByText("/permissions")).toBeTruthy();
    // 会话列：sessionId 原文
    expect(screen.getByText("sess-abc-1")).toBeTruthy();
    // 时间列：短时间格式（MM-dd HH:mm:ss），三行各一个
    expect(screen.getAllByText(/\d{2}-\d{2} \d{2}:\d{2}:\d{2}/)).toHaveLength(3);
  });

  it("空载荷 → 「暂无记录」空态，不渲染任何列表行", async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "inject_list_audit" ? { items: [] } : null
    );
    render(<AuditLogSection />);
    expect(await screen.findByText("No records yet")).toBeTruthy();
    expect(document.querySelectorAll("[data-audit-row]").length).toBe(0);
  });

  it("刷新按钮触发再次 invoke（调用计数 1 → 2）", async () => {
    render(<AuditLogSection />);
    expect(await screen.findByText("JARVIS 的 iPhone")).toBeTruthy();
    expect(auditCalls()).toBe(1);
    fireEvent.click(screen.getByRole("button", { name: /refresh/i }));
    await waitFor(() => expect(auditCalls()).toBe(2));
  });
});

// ==== P3 补锁：inject_list_audit 失败的可见性（错误态不伪装成空态；失败保留旧数据）====
describe("AuditLogSection 加载失败态（P3 补锁）", () => {
  function failAudit() {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "inject_list_audit") throw new Error("audit boom");
      return null;
    });
  }

  it("audit_load_error_state：失败且无数据 → 显示加载失败错误态（不伪装成空态）", async () => {
    failAudit();
    const errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      render(<AuditLogSection />);
      expect(await screen.findByTestId("audit-load-error").then((el) => el.textContent)).toContain(
        "Failed to load audit log"
      );
      // 不得渲染「暂无记录」空态（失败 ≠ 空数据）
      expect(screen.queryByText("No records yet")).toBeNull();
      expect(document.querySelectorAll("[data-audit-row]").length).toBe(0);
    } finally {
      errSpy.mockRestore();
    }
  });

  it("失败但有旧数据 → 保留旧数据（不渲染错误态顶掉列表，可重试）", async () => {
    render(<AuditLogSection />);
    expect(await screen.findByText("JARVIS 的 iPhone")).toBeTruthy();
    failAudit();
    const errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      fireEvent.click(screen.getByRole("button", { name: /refresh/i }));
      await waitFor(() => expect(auditCalls()).toBe(2));
      // 刷新失败：旧数据原样保留，错误行不出现（items 非空分支优先）
      expect(screen.getByText("JARVIS 的 iPhone")).toBeTruthy();
      expect(screen.getByText("Desktop-A")).toBeTruthy();
      expect(screen.queryByTestId("audit-load-error")).toBeNull();
      expect(document.querySelectorAll("[data-audit-row]").length).toBe(3);
    } finally {
      errSpy.mockRestore();
    }
  });

  it("失败态后刷新成功 → 错误态清除，列表恢复", async () => {
    failAudit();
    const errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      render(<AuditLogSection />);
      expect(await screen.findByTestId("audit-load-error")).toBeTruthy();
      invokeMock.mockImplementation(async (cmd: string) =>
        cmd === "inject_list_audit" ? { items: rowsOf() } : null
      );
      fireEvent.click(screen.getByRole("button", { name: /refresh/i }));
      expect(await screen.findByText("JARVIS 的 iPhone")).toBeTruthy();
      expect(screen.queryByTestId("audit-load-error")).toBeNull();
    } finally {
      errSpy.mockRestore();
    }
  });
});

// i18n 契约：AuditLogSection 源码引用的 settings.audit.* 键必须在 zh 与 en 两个
// locale 同齐备，且两 locale 的 settings.audit 键集相等（对齐 scripts/check-i18n 的子树版）
describe("AuditLogSection i18n zh/en 无缺键（M7 W5）", () => {
  // vitest jsdom 环境下 import.meta.url 非 file 协议，用进程 cwd（vitest 以仓库根启动）
  const root = process.cwd();
  const flat = (obj: Record<string, unknown>, prefix = ""): string[] =>
    Object.entries(obj).flatMap(([k, v]) =>
      typeof v === "object" && v !== null
        ? flat(v as Record<string, unknown>, `${prefix}${k}.`)
        : [`${prefix}${k}`]
    );

  it("源码引用键 zh/en 双语齐备，且两 locale 的 settings.audit 键集相等", () => {
    const zh = JSON.parse(readFileSync(path.join(root, "src/i18n/locales/zh.json"), "utf8"));
    const en = JSON.parse(readFileSync(path.join(root, "src/i18n/locales/en.json"), "utf8"));
    const zhKeys = new Set(flat(zh.settings.audit).map((k) => `settings.audit.${k}`));
    const enKeys = new Set(flat(en.settings.audit).map((k) => `settings.audit.${k}`));
    expect([...zhKeys].filter((k) => !enKeys.has(k))).toEqual([]);
    expect([...enKeys].filter((k) => !zhKeys.has(k))).toEqual([]);

    // 源码扫描：组件引用的键必须双 locale 存在（title/refresh/empty/time/device/session/action/summary）
    const text = readFileSync(
      path.join(root, "src/components/settings/AuditLogSection.tsx"),
      "utf8"
    );
    const used = new Set<string>();
    for (const m of text.matchAll(/settings\.audit\.([A-Za-z0-9_]+)/g)) used.add(m[0]);
    expect(used.size).toBeGreaterThanOrEqual(8);
    expect([...used].filter((k) => !zhKeys.has(k))).toEqual([]);
    expect([...used].filter((k) => !enKeys.has(k))).toEqual([]);
  });
});
