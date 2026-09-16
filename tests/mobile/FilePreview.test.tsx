import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import FilePreview from "@/mobile/FilePreview";
import type { Session } from "@/types/session";

// M3 Task 8：文件预览三态（文本 / markdown / 图片）+ 错误态 + 模式标记。
// fetch 全量 stub；jsdom 无真实高亮，只断言容器与文本存在。

function makeSession(): Session {
  return {
    id: "sess-1",
    agentType: "claude",
    projectName: "proj",
    projectPath: "/tmp/proj",
    title: null,
    gitBranch: null,
    githubUrl: null,
    status: "processing",
    lastMessage: null,
    lastMessageRole: null,
    lastActivityAt: "2026-09-15T00:00:00Z",
    pid: 1,
    cpuUsage: 0,
    activeSubagentCount: 0,
    form: "cli",
    jumpSupported: false,
    unread: false,
  };
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

/** /file 端点的 fetch stub（其余 URL 一律抛错，用例顺序错误立即暴露） */
function installFileFetch(impl: (url: string) => Response) {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (!url.includes("/file?")) throw new Error(`unexpected fetch: ${url}`);
      return impl(url);
    })
  );
}

function renderPreview(filePath = "/tmp/proj/main.rs", mode: "split" | "fullscreen" = "fullscreen") {
  return render(
    <FilePreview session={makeSession()} filePath={filePath} mode={mode} onClose={() => {}} />
  );
}

describe("FilePreview：文本 / markdown / 图片三态", () => {
  it("文本文件：<pre><code> 渲染内容（带 hljs 容器）", async () => {
    installFileFetch(
      () =>
        new Response(JSON.stringify({ content: "fn main() {}", mime: "text/rust" }), {
          status: 200,
        })
    );
    renderPreview();
    const code = await screen.findByTestId("preview-code");
    expect(code.textContent).toContain("fn main() {}");
    expect(code.querySelector("code.hljs")).toBeTruthy();
  });

  it("markdown 文件：ReactMarkdown 渲染（标题/加粗）", async () => {
    installFileFetch(
      () =>
        new Response(JSON.stringify({ content: "# 标题\n\n**加粗**", mime: "text/markdown" }), {
          status: 200,
        })
    );
    renderPreview("/tmp/proj/README.md");
    const md = await screen.findByTestId("preview-markdown");
    expect(md.textContent).toContain("标题");
    // Bug 5：markdown 渲染容器挂 .md-body 排版类（标题/列表语义样式）
    expect(md.classList.contains("md-body")).toBe(true);
    expect(md.querySelector("h1")).toBeTruthy();
    expect(screen.getByText("加粗").tagName).toBe("STRONG");
  });

  it("图片文件：<img> object URL，卸载时 revoke", async () => {
    const createObjectURL = vi.fn(() => "blob:mock-url");
    const revokeObjectURL = vi.fn();
    Object.defineProperty(URL, "createObjectURL", {
      value: createObjectURL,
      configurable: true,
      writable: true,
    });
    Object.defineProperty(URL, "revokeObjectURL", {
      value: revokeObjectURL,
      configurable: true,
      writable: true,
    });
    installFileFetch(
      () =>
        new Response(new Blob(["fake-png-bytes"]), {
          status: 200,
          headers: { "content-type": "image/png" },
        })
    );
    const { unmount } = renderPreview("/tmp/proj/pic.png");
    const img = (await screen.findByTestId("preview-image")) as HTMLImageElement;
    expect(img.getAttribute("src")).toBe("blob:mock-url");
    unmount();
    expect(revokeObjectURL).toHaveBeenCalledWith("blob:mock-url");
  });
});

describe("FilePreview：错误态与模式标记", () => {
  it("403（越界/超限/不存在不可区分）→「无法预览该文件」+ 重试按钮", async () => {
    installFileFetch(() => new Response("", { status: 403 }));
    renderPreview();
    expect((await screen.findByTestId("preview-error")).textContent).toContain(
      "无法预览该文件"
    );
    expect(screen.getByTestId("preview-retry")).toBeTruthy();
  });

  it("网络异常 → 通用加载失败文案（不给探测预言机）", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new TypeError("network down");
      })
    );
    renderPreview();
    expect((await screen.findByTestId("preview-error")).textContent).toContain("预览加载失败");
  });

  it("mode 双态：data-mode 标记 split / fullscreen", async () => {
    installFileFetch(
      () => new Response(JSON.stringify({ content: "x", mime: "text/plain" }), { status: 200 })
    );
    const { unmount } = renderPreview("/tmp/proj/a.txt", "split");
    expect((await screen.findByTestId("file-preview")).getAttribute("data-mode")).toBe("split");
    unmount();
    renderPreview("/tmp/proj/a.txt", "fullscreen");
    expect((await screen.findByTestId("file-preview")).getAttribute("data-mode")).toBe(
      "fullscreen"
    );
  });
});
