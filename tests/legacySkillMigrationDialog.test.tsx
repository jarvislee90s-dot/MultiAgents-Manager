// tests/legacySkillMigrationDialog.test.tsx — 遗留 codex 技能链接迁移对话框（spec §4.3/§6/§7）
// 覆盖：detect 命中 → 清单与双按钮可见；取消 = migrate 零调用；迁移/保留 invoke 入参；
// 执行后结果视图；zh/en 词条存在性
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { LegacySkillMigrationDialog } from "@/components/resources/LegacySkillMigrationDialog";
import {
  __resetLegacyMigrationDetectionForTest,
  useLegacySkillMigration,
} from "@/hooks/useLegacySkillMigration";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

// tests/setup.ts 未初始化 i18n，显式引入并固定中文（对话框文案按 zh 断言）
import i18n from "@/i18n";
import enJson from "@/i18n/locales/en.json";
import zhJson from "@/i18n/locales/zh.json";

beforeAll(async () => {
  await i18n.changeLanguage("zh");
});

// 测试替身：真实 hook（mount 时 detect）+ 真实对话框，等价于 home.tsx 的挂载方式
function Harness() {
  const migration = useLegacySkillMigration();
  return <LegacySkillMigrationDialog migration={migration} />;
}

const MIGRATION_REPORTS = [
  { name: "skill-a", status: "ok", detail: null },
  { name: "skill-b", status: "skipped", detail: "目标已存在同名目录" },
];

beforeEach(() => {
  // 模块级 once-flag 会被同文件多用例共享，逐用例重置（review Minor：SPA 重入防护）
  __resetLegacyMigrationDetectionForTest();
  window.localStorage.clear();
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === "detect_legacy_agents_links") return ["skill-a", "skill-b"];
    if (cmd === "migrate_legacy_agents_links") return MIGRATION_REPORTS;
    return [];
  });
});

describe("LegacySkillMigrationDialog", () => {
  it("detect 命中 → 清单两项可见、双按钮中文文案可见", async () => {
    render(<Harness />);
    expect(await screen.findByText("skill-a")).toBeInTheDocument();
    expect(screen.getByText("skill-b")).toBeInTheDocument();
    // 描述带数量插值（detect 返回 2 条）
    expect(screen.getByText(/发现 2 个由 MAM 创建/)).toBeInTheDocument();
    // 双按钮 + 语义说明 + 取消（zh 词条）
    expect(screen.getByRole("button", { name: "迁移到 .codex/skills" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "保留为共享" })).toBeInTheDocument();
    expect(screen.getByText("暂不处理")).toBeInTheDocument();
    // mount 即触发过 detect
    expect(invokeMock).toHaveBeenCalledWith("detect_legacy_agents_links");
  });

  it("点击取消 → migrate 的 invoke 未被调用，对话框关闭（spec §6 关闭语义）", async () => {
    render(<Harness />);
    fireEvent.click(await screen.findByText("暂不处理"));
    await waitFor(() =>
      expect(screen.queryByText("受影响的技能")).not.toBeInTheDocument()
    );
    expect(invokeMock).not.toHaveBeenCalledWith(
      "migrate_legacy_agents_links",
      expect.anything()
    );
  });

  it("点击「迁移到 .codex/skills」→ 以 mode: migrate 调用并展示逐条结果", async () => {
    render(<Harness />);
    fireEvent.click(await screen.findByRole("button", { name: "迁移到 .codex/skills" }));
    // invoke 入参断言（后端契约：mode: "migrate"）
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("migrate_legacy_agents_links", {
        mode: "migrate",
      })
    );
    // 结果视图：逐条 name + status + detail（name 与状态分属相邻节点，分开断言）
    expect(await screen.findByText("处理结果")).toBeInTheDocument();
    expect(screen.getByText("skill-a")).toBeInTheDocument();
    expect(screen.getByText("成功")).toBeInTheDocument();
    expect(screen.getByText("skill-b")).toBeInTheDocument();
    expect(screen.getByText("已跳过")).toBeInTheDocument();
    expect(screen.getByText("目标已存在同名目录")).toBeInTheDocument();
  });

  it("点击「保留为共享」→ 以 mode: keep 调用", async () => {
    render(<Harness />);
    fireEvent.click(await screen.findByRole("button", { name: "保留为共享" }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("migrate_legacy_agents_links", {
        mode: "keep",
      })
    );
  });
});

describe("i18n 词条存在性（zh/en 双语同构）", () => {
  const REQUIRED_KEYS = [
    "title",
    "description",
    "listTitle",
    "migrateBtn",
    "migrateDesc",
    "keepBtn",
    "keepDesc",
    "cancel",
    "resultTitle",
    "statusOk",
    "statusSkipped",
    "statusError",
    "runFailed",
    "close",
  ] as const;

  it("zh/en 两 locale 均含 resources.migration 全部新键", () => {
    const zh = zhJson.resources.migration as Record<string, unknown>;
    const en = enJson.resources.migration as Record<string, unknown>;
    for (const key of REQUIRED_KEYS) {
      expect(zh[key]).toBeDefined();
      expect(en[key]).toBeDefined();
    }
  });
});
