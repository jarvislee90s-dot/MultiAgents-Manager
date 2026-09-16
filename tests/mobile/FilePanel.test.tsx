// M3+ 文件面板（计划 Task 3）：纯函数 + 组件行为矩阵。
// 零真实数据：条目夹具全部合成；纯展示组件，零网络（点击回调由测试断言）。
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import FilePanel, { FILE_SCOPES, fileKindOf } from "@/mobile/FilePanel";
import type { SessionFileEntry } from "@/mobile/api";

afterEach(cleanup);

/** 条目夹具（后端 FileEntry camelCase 契约） */
function entry(path: string, over: Partial<SessionFileEntry> = {}): SessionFileEntry {
  return { path, lastSeq: 1, lastTs: 1_700_000_000_000, hits: 1, ...over };
}

/** 渲染面板（默认回调全部注入 mock） */
function renderPanel(
  entries: SessionFileEntry[],
  over: { truncated?: boolean; scope?: number; loading?: boolean } = {}
) {
  const onOpenFile = vi.fn();
  const onScopeChange = vi.fn();
  const onModeChange = vi.fn();
  const onClose = vi.fn();
  render(
    <FilePanel
      entries={entries}
      truncated={over.truncated ?? false}
      scope={over.scope ?? 200}
      loading={over.loading ?? false}
      mode="fullscreen"
      onScopeChange={onScopeChange}
      onOpenFile={onOpenFile}
      onModeChange={onModeChange}
      onClose={onClose}
    />
  );
  return { onOpenFile, onScopeChange, onModeChange, onClose };
}

describe("FilePanel 纯函数：fileKindOf 扩展名判定", () => {
  it("文档档：md / markdown / txt（大小写不敏感）", () => {
    expect(fileKindOf("/p/README.md")).toBe("doc");
    expect(fileKindOf("/p/notes.markdown")).toBe("doc");
    expect(fileKindOf("/p/a.txt")).toBe("doc");
    expect(fileKindOf("/p/UPPER.MD")).toBe("doc");
  });

  it("图片档：png/jpg/jpeg/gif/webp/svg/bmp（用户裁决 5 白名单）", () => {
    for (const ext of ["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp"]) {
      expect(fileKindOf(`/p/x.${ext}`)).toBe("image");
      expect(fileKindOf(`/p/x.${ext.toUpperCase()}`)).toBe("image");
    }
  });

  it("代码与无扩展名归 other（代码不在专属档，仅在「全部」可见——用户裁决 5）", () => {
    expect(fileKindOf("/p/main.rs")).toBe("other");
    expect(fileKindOf("/p/app.tsx")).toBe("other");
    expect(fileKindOf("/p/Makefile")).toBe("other");
    expect(fileKindOf("/p/.gitignore")).toBe("other"); // 点开头无扩展名
    expect(fileKindOf("D:\\proj\\src\\m.rs")).toBe("other"); // Windows 分隔符
  });

  it("档位常量：三档且顶档 1000（= SessionDetail.MAX_LIMIT）", () => {
    expect([...FILE_SCOPES]).toEqual([200, 500, 1000]);
  });
});

describe("FilePanel 列表渲染", () => {
  it("渲染顺序 = entries 顺序（后端已按 lastSeq 降序，前端不重排）", () => {
    renderPanel([
      entry("/p/late.rs", { lastSeq: 9 }),
      entry("/p/mid.rs", { lastSeq: 5 }),
      entry("/p/early.rs", { lastSeq: 1 }),
    ]);
    const names = [0, 1, 2].map(
      (i) => within(screen.getByTestId(`file-row-${i}`)).getAllByRole("button")[0].textContent
    );
    expect(names[0]).toContain("late.rs");
    expect(names[1]).toContain("mid.rs");
    expect(names[2]).toContain("early.rs");
  });

  it("行信息齐全：文件名（主行）+ 目录前缀 + 相对时间 + hits 徽标", () => {
    renderPanel([entry("/tmp/proj/src/app.rs", { hits: 3 })]);
    const row = screen.getByTestId("file-row-0");
    expect(within(row).getByTestId("file-row-0-open").textContent).toContain("app.rs");
    expect(row.textContent).toContain("/tmp/proj/src");
    expect(row.textContent).toContain("3×");
  });

  it("hits=1 不显示徽标；lastTs=null 显示占位符 —", () => {
    renderPanel([entry("/p/a.rs", { hits: 1 }), entry("/p/b.rs", { lastTs: null, lastSeq: 3 })]);
    expect(screen.getByTestId("file-row-0").textContent).not.toContain("×");
    expect(screen.getByTestId("file-row-1").textContent).toContain("—");
  });

  it("主行点击 → onOpenFile 收到完整 path", () => {
    const { onOpenFile } = renderPanel([entry("/tmp/proj/src/app.rs")]);
    fireEvent.click(screen.getByTestId("file-row-0-open"));
    expect(onOpenFile).toHaveBeenCalledWith("/tmp/proj/src/app.rs");
  });

  it("空态：该范围内未发现文件", () => {
    renderPanel([]);
    expect(screen.getByTestId("panel-empty").textContent).toContain("该范围内未发现文件");
  });

  it("加载态：有旧条目时保留列表（半透明），不闪空态", () => {
    renderPanel([entry("/p/a.rs")], { loading: true });
    expect(screen.getByTestId("file-row-0")).toBeTruthy();
    expect(screen.queryByTestId("panel-empty")).toBeNull();
    expect(screen.getByText("加载中…")).toBeTruthy();
  });
});

describe("FilePanel chips 类型过滤（用户裁决 5）", () => {
  const mixed = [
    entry("/p/doc.md", { lastSeq: 5 }),
    entry("/p/pic.png", { lastSeq: 4 }),
    entry("/p/code.rs", { lastSeq: 3 }),
  ];

  it("默认「全部」：三类都可见（代码文件仅在全部）", () => {
    renderPanel(mixed);
    expect(screen.getByText("doc.md")).toBeTruthy();
    expect(screen.getByText("pic.png")).toBeTruthy();
    expect(screen.getByText("code.rs")).toBeTruthy();
  });

  it("切「文档」：仅 md 可见，图片与代码隐藏", () => {
    renderPanel(mixed);
    fireEvent.click(screen.getByTestId("file-chip-doc"));
    expect(screen.getByText("doc.md")).toBeTruthy();
    expect(screen.queryByText("pic.png")).toBeNull();
    expect(screen.queryByText("code.rs")).toBeNull();
  });

  it("切「图片」：仅 png 可见", () => {
    renderPanel(mixed);
    fireEvent.click(screen.getByTestId("file-chip-image"));
    expect(screen.getByText("pic.png")).toBeTruthy();
    expect(screen.queryByText("doc.md")).toBeNull();
    expect(screen.queryByText("code.rs")).toBeNull();
  });

  it("过滤后无匹配 → 空态（不与「范围内无文件」混淆）", () => {
    renderPanel([entry("/p/code.rs")]);
    fireEvent.click(screen.getByTestId("file-chip-image"));
    expect(screen.getByTestId("panel-empty")).toBeTruthy();
  });
});

describe("FilePanel 档位卡片（用户裁决 3）", () => {
  it("三档常显、当前高亮、点击回调 onScopeChange", () => {
    const { onScopeChange } = renderPanel([entry("/p/a.rs")], { scope: 500 });
    for (const s of FILE_SCOPES) {
      expect(screen.getByTestId(`file-scope-${s}`)).toBeTruthy();
    }
    expect(screen.getByTestId("file-scope-500").getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByTestId("file-scope-200").getAttribute("aria-pressed")).toBe("false");
    fireEvent.click(screen.getByTestId("file-scope-1000"));
    expect(onScopeChange).toHaveBeenCalledWith(1000);
  });

  it("truncated 且未到顶 → 显示提示", () => {
    renderPanel([entry("/p/a.rs")], { truncated: true, scope: 200 });
    expect(screen.getByTestId("panel-truncated-hint").textContent).toContain("还有更早文件");
  });

  it("scope=1000 到顶 → 不显示提示（即使 truncated 为真）", () => {
    renderPanel([entry("/p/a.rs")], { truncated: true, scope: 1000 });
    expect(screen.queryByTestId("panel-truncated-hint")).toBeNull();
  });

  it("未 truncated → 不显示提示", () => {
    renderPanel([entry("/p/a.rs")], { truncated: false, scope: 200 });
    expect(screen.queryByTestId("panel-truncated-hint")).toBeNull();
  });
});

describe("FilePanel 目录路径点击弹全路径（2026-09-16 用户裁决）", () => {
  const longPath = "/very/long/directory/prefix/that/gets/truncated/in/the/list/view/sub";

  it("点次行目录 → 弹层显示完整路径（手机与电脑逻辑一致）", () => {
    renderPanel([entry(`${longPath}/app.rs`)]);
    // 弹层初始不存在
    expect(screen.queryByTestId("path-popover")).toBeNull();
    fireEvent.click(screen.getByTestId("file-row-0-dir"));
    const pop = screen.getByTestId("path-popover");
    expect(pop.textContent).toContain(longPath);
  });

  it("弹层可关闭（再点目录 / 点关闭按钮）", () => {
    renderPanel([entry(`${longPath}/app.rs`)]);
    fireEvent.click(screen.getByTestId("file-row-0-dir"));
    expect(screen.getByTestId("path-popover")).toBeTruthy();
    fireEvent.click(screen.getByTestId("path-popover-close"));
    expect(screen.queryByTestId("path-popover")).toBeNull();
  });

  it("无目录前缀的行不该有可点目录按钮（避免空弹层）", () => {
    renderPanel([entry("app.rs")]);
    expect(screen.queryByTestId("file-row-0-dir")).toBeNull();
  });
});

describe("FilePanel 头部操作", () => {
  it("关闭按钮回调 onClose；切换器回调 onModeChange", () => {
    const { onClose, onModeChange } = renderPanel([entry("/p/a.rs")]);
    fireEvent.click(screen.getByTestId("panel-close"));
    expect(onClose).toHaveBeenCalled();
    fireEvent.click(screen.getByTestId("preview-toggle-split-h"));
    expect(onModeChange).toHaveBeenCalledWith("split-h");
  });
});
