import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SessionDetail from "@/mobile/SessionDetail";
import type { SessionMessage } from "@/mobile/api";
import type { Session } from "@/types/session";

// M3 Task 8：ZCode 式会话详情页渲染矩阵。fetch 全量 stub（盖过 setup.ts 的 msw），
// 按 URL 分路到 messages / session-files / file 三端点；jsdom 无真实高亮，
// 只断言容器与文本存在（遵循任务约束）。

function makeSession(overrides: Partial<Session> = {}): Session {
  return {
    id: "sess-1",
    agentType: "claude",
    projectName: "proj",
    projectPath: "/tmp/proj",
    title: "修 bug",
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
    ...overrides,
  };
}

function msg(
  overrides: Partial<SessionMessage> & Pick<SessionMessage, "seq" | "kind" | "content">
): SessionMessage {
  return {
    role: overrides.kind === "user" ? "user" : "assistant",
    ts: 1000,
    collapsed: overrides.kind === "thinking" || overrides.kind === "tool-call",
    ...overrides,
  };
}

interface Routes {
  messages?: SessionMessage[];
  messagesStatus?: number;
  messagesNetworkFail?: boolean;
  /** Bug 1（M3 验收）：后端头部截断标记，随 /session-messages 载荷返回 */
  truncated?: boolean;
  files?: string[];
  fileContent?: string;
  fileMime?: string;
  fileStatus?: number;
}

let routes: Routes;
let fetchMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  routes = {};
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

/** 按 URL 分路的 fetch stub，返回 mock 供断言调用参数 */
function installFetch() {
  fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes("/session-messages")) {
      if (routes.messagesNetworkFail) throw new TypeError("network down");
      if (routes.messagesStatus) {
        return new Response("gone", { status: routes.messagesStatus });
      }
      return new Response(
        JSON.stringify({ messages: routes.messages ?? [], truncated: routes.truncated === true }),
        { status: 200 }
      );
    }
    if (url.includes("/session-files")) {
      return new Response(JSON.stringify({ files: routes.files ?? [] }), { status: 200 });
    }
    if (url.includes("/file?")) {
      if (routes.fileStatus) return new Response("no", { status: routes.fileStatus });
      return new Response(
        JSON.stringify({ content: routes.fileContent ?? "", mime: routes.fileMime ?? "text/plain" }),
        { status: 200 }
      );
    }
    throw new Error(`unexpected fetch: ${url}`);
  });
  vi.stubGlobal("fetch", fetchMock);
}

describe("SessionDetail：消息渲染与折叠交互（P9）", () => {
  it("渲染对话：assistant 正文走 markdown，user 直显", async () => {
    installFetch();
    routes.messages = [
      msg({ seq: 0, kind: "user", content: "帮我看看" }),
      msg({ seq: 1, kind: "assistant", content: "已修复，重点在 **并发** 处" }),
    ];
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    expect(await screen.findByText("帮我看看")).toBeTruthy();
    // markdown：**并发** → <strong>（jsdom 无真实高亮，断言元素与文本即可）
    const strong = await screen.findByText("并发");
    expect(strong.tagName).toBe("STRONG");
    // 页头：项目名 + 工具名
    expect(screen.getByText("proj")).toBeTruthy();
    expect(screen.getByText(/Claude/)).toBeTruthy();
  });

  it("运行中：thinking/tool-call 默认折叠，点击展开显示 toolName+toolArgs", async () => {
    installFetch();
    routes.messages = [
      msg({ seq: 0, kind: "user", content: "查一下" }),
      msg({ seq: 1, kind: "thinking", content: "内部思考内容" }),
      msg({
        seq: 2,
        kind: "tool-call",
        content: "调用 Bash",
        toolName: "Bash",
        toolArgs: '{"command":"ls"}',
      }),
      msg({ seq: 3, kind: "assistant", content: "结论" }),
    ];
    render(<SessionDetail session={makeSession({ status: "processing" })} onBack={() => {}} />);
    expect(await screen.findByText("结论")).toBeTruthy();
    // 默认折叠：内容不可见，折叠头可见
    expect(screen.queryByText("内部思考内容")).toBeNull();
    expect(screen.queryByText('{"command":"ls"}')).toBeNull();
    expect(screen.getByTestId("msg-1-toggle").textContent).toContain("思考过程");
    expect(screen.getByTestId("msg-2-toggle").textContent).toContain("调用 Bash");
    // 点击展开：thinking 内容 + toolArgs 均出现，aria-expanded 翻转
    fireEvent.click(screen.getByTestId("msg-1-toggle"));
    expect(screen.getByText("内部思考内容")).toBeTruthy();
    fireEvent.click(screen.getByTestId("msg-2-toggle"));
    expect(screen.getByText('{"command":"ls"}')).toBeTruthy();
    expect(screen.getByTestId("msg-2-toggle").getAttribute("aria-expanded")).toBe("true");
    // 再点回收起
    fireEvent.click(screen.getByTestId("msg-2-toggle"));
    expect(screen.queryByText('{"command":"ls"}')).toBeNull();
  });

  it("idle 会话（总结模式）：过程消息与更早 assistant 自动折叠，只显最后 assistant 总结", async () => {
    installFetch();
    routes.messages = [
      msg({ seq: 0, kind: "user", content: "查一下" }),
      msg({ seq: 1, kind: "thinking", content: "内部思考内容" }),
      msg({ seq: 2, kind: "assistant", content: "中间回复" }),
      msg({ seq: 3, kind: "tool-call", content: "调用 Grep", toolName: "Grep" }),
      msg({ seq: 4, kind: "assistant", content: "最终总结" }),
    ];
    render(<SessionDetail session={makeSession({ status: "idle" })} onBack={() => {}} />);
    // user 与最后 assistant 直显；过程消息与中间 assistant 折叠
    expect(await screen.findByText("查一下")).toBeTruthy();
    expect(screen.getByText("最终总结")).toBeTruthy();
    expect(screen.queryByText("内部思考内容")).toBeNull();
    expect(screen.queryByText("中间回复")).toBeNull();
    // tool-call 只剩折叠头（标签=「调用 Grep」），无展开内容
    expect(screen.getByTestId("msg-3-toggle").textContent).toContain("调用 Grep");
    // 更早 assistant 折叠头标签 =「更早的回复」，点击展开
    fireEvent.click(screen.getByTestId("msg-2-toggle"));
    expect(screen.getByText("中间回复")).toBeTruthy();
    // 过程消息同样可展开
    fireEvent.click(screen.getByTestId("msg-1-toggle"));
    expect(screen.getByText("内部思考内容")).toBeTruthy();
  });

  it("加载更早消息：点击以更大 limit 整页重拉（200 → 400）", async () => {
    installFetch();
    routes.messages = Array.from({ length: 200 }, (_, i) =>
      msg({ seq: i, kind: "user", content: `m${i}` })
    );
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    expect(await screen.findByText("m199")).toBeTruthy();
    fireEvent.click(screen.getByTestId("load-more"));
    // timeout 加固（flaky 修复）：mock 本身同步 resolve（无真实 timer 需求），
    // 但 59 文件并行时 fetch mock→state 更新链可能被资源竞争拉长，默认 1s 不够
    await waitFor(
      () => {
        const called400 = fetchMock.mock.calls.some((c: unknown[]) =>
          String(c[0]).includes("limit=400")
        );
        expect(called400).toBe(true);
      },
      { timeout: 3000 }
    );
  }, 15000); // 用例级 timeout 加固（flaky 修复②）：60 文件并行时 waitFor 内的
  // mock→state 链可能吃满默认 5s 用例预算（b7a407d 只加固了 waitFor 自身）

  it("Bug 1：truncated=true 且条数 < limit 时仍显示加载更早（胖会话字节截断）", async () => {
    installFetch();
    // 胖 JSONL 单行吃掉整个 512KB 字节窗：返回条数远小于 limit，但头部被切
    routes.messages = Array.from({ length: 5 }, (_, i) =>
      msg({ seq: i, kind: "user", content: `m${i}` })
    );
    routes.truncated = true;
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    expect(await screen.findByText("m4")).toBeTruthy();
    // 旧实现条件 length >= limit 恒 false → 按钮死功能；新实现 truncated 也能触发
    expect(screen.getByTestId("load-more")).toBeTruthy();
    // 点击 → 以更大 limit 重拉（后端字节窗随 limit 放大，更早内容可达）
    fireEvent.click(screen.getByTestId("load-more"));
    await waitFor(
      () => {
        const called400 = fetchMock.mock.calls.some((c: unknown[]) =>
          String(c[0]).includes("limit=400")
        );
        expect(called400).toBe(true);
      },
      { timeout: 3000 }
    );
  }, 15000); // 用例级 timeout 加固：同上
});

describe("SessionDetail：文件链接化与预览联动", () => {
  it("文件路径渲染为链接：点击开全屏预览，可切分屏，关闭回到对话", async () => {
    installFetch();
    routes.messages = [
      msg({ seq: 0, kind: "assistant", content: "改了 /tmp/proj/src/app.rs 请看" }),
    ];
    routes.files = ["/tmp/proj/src/app.rs"];
    routes.fileContent = "fn main() {}";
    routes.fileMime = "text/rust";
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    // 链接化按钮出现在正文里（content 中出现的已知路径）
    fireEvent.click(await screen.findByTestId("file-link"));
    // 默认全屏浮层
    const preview = await screen.findByTestId("file-preview");
    expect(preview.getAttribute("data-mode")).toBe("fullscreen");
    expect((await screen.findByTestId("preview-code")).textContent).toContain("fn main() {}");
    // 切分屏：data-mode 翻转（split = 上对话下文件由布局类承担，此处锁语义切换）
    fireEvent.click(screen.getByTestId("preview-mode-split"));
    expect(screen.getByTestId("file-preview").getAttribute("data-mode")).toBe("split");
    // 关闭预览：对话仍在（同一组件树内状态保持）
    fireEvent.click(screen.getByTestId("preview-close"));
    expect(screen.queryByTestId("file-preview")).toBeNull();
    expect(screen.getByText("改了", { exact: false })).toBeTruthy();
  });

  it("Bug 2：全屏预览态下切换按钮可达（FilePreview 页头），一键切分屏出分屏容器", async () => {
    installFetch();
    routes.messages = [
      msg({ seq: 0, kind: "assistant", content: "改了 /tmp/proj/src/app.rs 请看" }),
    ];
    routes.files = ["/tmp/proj/src/app.rs"];
    routes.fileContent = "fn main() {}";
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    fireEvent.click(await screen.findByTestId("file-link"));
    // 默认全屏浮层（jsdom 无 matchMedia → 窄屏默认；宽屏自适应由 openFile 判定）
    const preview = await screen.findByTestId("file-preview");
    expect(preview.getAttribute("data-mode")).toBe("fullscreen");
    // 全屏浮层 fixed inset-0 盖住 SessionDetail 页头 → 页头切换器不可达；
    // FilePreview 页头自带的切换控件是全屏态唯一入口
    fireEvent.click(screen.getByTestId("preview-toggle-split"));
    expect(screen.getByTestId("file-preview").getAttribute("data-mode")).toBe("split");
    // 分屏容器出现（上对话下文件布局，Task 8 裁决）
    expect(screen.getByTestId("split-container")).toBeTruthy();
    // 分屏态可一键切回全屏
    fireEvent.click(screen.getByTestId("preview-toggle-fullscreen"));
    expect(screen.getByTestId("file-preview").getAttribute("data-mode")).toBe("fullscreen");
  });

  it("Bug 2 顺手项：宽屏（matchMedia ≥768px）打开文件自动进分屏，窄屏默认全屏", async () => {
    installFetch();
    routes.messages = [
      msg({ seq: 0, kind: "assistant", content: "改了 /tmp/proj/src/app.rs 请看" }),
    ];
    routes.files = ["/tmp/proj/src/app.rs"];
    routes.fileContent = "fn main() {}";
    // jsdom 未实现 matchMedia：装 shim（theme.test.ts 同款模式）
    Object.defineProperty(window, "matchMedia", {
      writable: true,
      value: (q: string) => ({ matches: q.includes("min-width"), media: q }),
    });
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    fireEvent.click(await screen.findByTestId("file-link"));
    expect((await screen.findByTestId("file-preview")).getAttribute("data-mode")).toBe("split");
    expect(screen.getByTestId("split-container")).toBeTruthy();
  });

  it("未知路径不出链接：files 为空时正文原样", async () => {
    installFetch();
    routes.messages = [msg({ seq: 0, kind: "assistant", content: "见 /tmp/other/x.rs" })];
    routes.files = [];
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    await screen.findByText(/\/tmp\/other\/x\.rs/);
    expect(screen.queryByTestId("file-link")).toBeNull();
  });
});

describe("SessionDetail：错误态与手动刷新", () => {
  it("404 →「无法读取该会话内容」+ 重试可恢复", async () => {
    installFetch();
    routes.messagesStatus = 404;
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    expect((await screen.findByTestId("detail-error")).textContent).toContain(
      "无法读取该会话内容"
    );
    routes.messagesStatus = undefined;
    routes.messages = [msg({ seq: 0, kind: "user", content: "恢复后可见" })];
    fireEvent.click(screen.getByTestId("detail-retry"));
    expect(await screen.findByText("恢复后可见")).toBeTruthy();
  });

  it("网络异常 → 加载失败 + 重试；刷新按钮重拉不自动轮询", async () => {
    installFetch();
    routes.messagesNetworkFail = true;
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    expect((await screen.findByTestId("detail-error")).textContent).toContain("加载失败");
    routes.messagesNetworkFail = false;
    routes.messages = [msg({ seq: 0, kind: "user", content: "刷新结果" })];
    fireEvent.click(screen.getByTestId("detail-retry"));
    expect(await screen.findByText("刷新结果")).toBeTruthy();
    // 手动刷新（refresh 按钮）同样走重拉；timeout 加固同上（并行资源竞争双保险）
    fireEvent.click(screen.getByTestId("detail-refresh"));
    await waitFor(
      () => {
        expect(fetchMock.mock.calls.length).toBeGreaterThanOrEqual(4);
      },
      { timeout: 3000 }
    );
  });
});
