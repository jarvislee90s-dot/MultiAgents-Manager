import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SessionDetail from "@/mobile/SessionDetail";
import type { SessionFileEntry, SessionMessage } from "@/mobile/api";
import { BOOKMARK_COLORS, clearBookmarks, messageAnchor } from "@/mobile/bookmarks";
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

/** M3+ 文件面板条目夹具（后端 FileEntry camelCase 契约） */
function fileEntry(path: string, over: Partial<SessionFileEntry> = {}): SessionFileEntry {
  return { path, lastSeq: 1, lastTs: 1000, hits: 1, modified: true, ...over };
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
  files?: SessionFileEntry[];
  filesTruncated?: boolean;
  fileContent?: string;
  fileMime?: string;
  fileStatus?: number;
  /** 评审 M4：/session-open 路由（缺省 200 opening；failed 载荷驱动 C1 分诊测试）；
   *  sessionOpenStatus 驱动 404 错误码分支（no_cwd / no_resume_command / no_session） */
  sessionOpen?: { status?: string; error?: string };
  sessionOpenStatus?: number;
  /** 发送能力探测（MessageComposer 挂载即拉）：可注入态夹具。
   *  组件在 infoReady 前 / sendInfo 为 null 时自隐——缺省给可注入，使 composer 渲染 */
  sendInfo?: { injectable: boolean; channels: string[]; visibility: string };
  /** 审批选项卡数据源（ApproveCard 挂载即拉）：available 为假时卡自隐——
   *  分屏挂载断言需给 available=true，否则断言的是「卡自隐」而非「没挂载」 */
  approveOptions?: {
    available: boolean;
    options: { id: string; label: string }[];
    verifiedWith: string;
    currentVersion: string | null;
    drift: boolean;
    reason?: string;
  };
}

let routes: Routes;
let fetchMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  routes = {};
  // 书签 store 用例间隔离（模块级单例 + localStorage 镜像，不清理会串场——
  // 如上一个用例占用了某颜色，下一个用例的调色板里该色就变置灰不可点）
  clearBookmarks("sess-1");
  window.localStorage.removeItem("mam-bookmarks");
  // matchMedia 用例间隔离：默认「无 matchMedia」= 窄屏语义（jsdom 原生行为），
  // 需要宽屏的用例自行安装后由 afterEach 还原（旧版仅少数用例安装，
  // 泄漏会让后续用例误判宽屏——如面板默认布局用例）
  Object.defineProperty(window, "matchMedia", {
    writable: true,
    configurable: true,
    value: undefined,
  });
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
      return new Response(
        JSON.stringify({ files: routes.files ?? [], truncated: routes.filesTruncated === true }),
        { status: 200 }
      );
    }
    if (url.includes("/host")) {
      // 书签恢复（M3+）依赖 bootId：默认给固定值（用例间由 beforeEach 清 store）
      return new Response(
        JSON.stringify({
          host: { name: "n", platform: "windows", version: "0", bootId: "boot-test" },
          enabledTools: [],
        }),
        { status: 200 }
      );
    }
    if (url.includes("/session-open")) {
      return new Response(JSON.stringify(routes.sessionOpen ?? { status: "opening" }), {
        status: routes.sessionOpenStatus ?? 200,
      });
    }
    if (url.includes("/session-send-info")) {
      // MessageComposer 挂载即拉；缺省给可注入，使分屏态 composer 真正渲染出来
      // （sendInfo 为 null 时组件自隐，会把「分屏有没有挂载」的断言变成假阴性）
      return new Response(
        JSON.stringify(
          routes.sendInfo ?? { injectable: true, channels: ["tmux"], visibility: "realtime" }
        ),
        { status: 200 }
      );
    }
    if (url.includes("/session-approve-options")) {
      // ApproveCard 挂载即拉；缺省给 available=false 无 reason（卡自隐，不改既有用例渲染）。
      // 「分屏红卡挂载」用例须显式给 available=true——否则断言的是卡自隐而非没挂载
      return new Response(
        JSON.stringify(
          routes.approveOptions ?? {
            available: false,
            options: [],
            verifiedWith: "test",
            currentVersion: null,
            drift: false,
          }
        ),
        { status: 200 }
      );
    }
    if (url.includes("/file?")) {
      if (routes.fileStatus) return new Response("no", { status: routes.fileStatus });
      return new Response(
        JSON.stringify({
          content: routes.fileContent ?? "",
          mime: routes.fileMime ?? "text/plain",
        }),
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
    routes.files = [fileEntry("/tmp/proj/src/app.rs")];
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
    // —— 切换器唯一实例在预览页头（2026-09-16 裁决：详情页头那份已删）
    fireEvent.click(screen.getByTestId("preview-toggle-split"));
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
    routes.files = [fileEntry("/tmp/proj/src/app.rs")];
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
    routes.files = [fileEntry("/tmp/proj/src/app.rs")];
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

  it("Bug 5：markdown 标题/列表渲染结构化元素且容器挂 md-body 排版类", async () => {
    installFetch();
    routes.messages = [
      msg({
        seq: 0,
        kind: "assistant",
        content:
          "## 小节标题\n\n- 第一项\n- 第二项\n\n1. 有序\n\n> 引用\n\n| 列A | 列B |\n|---|---|\n| a | b |",
      }),
    ];
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    expect(await screen.findByText("小节标题")).toBeTruthy();
    // 结构化元素存在（ReactMarkdown 本就产出；视觉拍平是 CSS 层问题）
    const h2 = screen.getByText("小节标题").closest("h2");
    expect(h2).toBeTruthy();
    const firstLi = screen.getByText("第一项").closest("li");
    expect(firstLi).toBeTruthy();
    expect(firstLi!.closest("ul")).toBeTruthy();
    expect(screen.getByText("有序").closest("ol")).toBeTruthy();
    expect(screen.getByText("引用").closest("blockquote")).toBeTruthy();
    expect(document.querySelector(".md-body table")).toBeTruthy();
    // Bug 5 修复锚点：渲染容器必须挂 .md-body 排版类（mobile.css 提供
    // 标题分级/列表符号/引用边框/表格边框，对抗 preflight 重置）
    expect(h2!.closest(".md-body")).toBeTruthy();
  });

  it("Bug 8：总结模式提示行计数正确，展开全部/收起全部生效；运行中不出现", async () => {
    installFetch();
    routes.messages = [
      msg({ seq: 0, kind: "user", content: "查一下" }),
      msg({ seq: 1, kind: "thinking", content: "内部思考内容" }),
      msg({ seq: 2, kind: "assistant", content: "中间回复" }),
      msg({ seq: 3, kind: "tool-call", content: "调用 Grep", toolName: "Grep" }),
      msg({ seq: 4, kind: "assistant", content: "最终总结" }),
    ];
    const { unmount } = render(
      <SessionDetail session={makeSession({ status: "idle" })} onBack={() => {}} />
    );
    await screen.findByText("最终总结");
    // 计数 = 当前被折叠的可折叠条数（thinking / 中间 assistant / tool-call = 3；
    // user 直显、最终 assistant 总结直显，不计入）
    const banner = screen.getByTestId("summary-banner");
    expect(banner.textContent).toContain("总结模式");
    expect(banner.textContent).toContain("已折叠 3 条过程消息");
    // 展开全部：过程消息全部可见，计数归零，收起全部出现
    fireEvent.click(screen.getByTestId("expand-all"));
    expect(screen.getByText("内部思考内容")).toBeTruthy();
    expect(screen.getByText("中间回复")).toBeTruthy();
    expect(screen.getByTestId("summary-banner").textContent).toContain("已折叠 0 条过程消息");
    // 收起全部：恢复默认折叠语义
    fireEvent.click(screen.getByTestId("collapse-all"));
    expect(screen.queryByText("内部思考内容")).toBeNull();
    expect(screen.getByTestId("summary-banner").textContent).toContain("已折叠 3 条过程消息");
    unmount();
    // 运行中模式：折叠是 wire 语义，不出现总结提示行
    render(<SessionDetail session={makeSession({ status: "processing" })} onBack={() => {}} />);
    expect(await screen.findByText("查一下")).toBeTruthy();
    expect(screen.queryByTestId("summary-banner")).toBeNull();
  });

  it("需求 1：横向分屏（左对话右文件）可用，与上下分屏/全屏三态互通", async () => {
    installFetch();
    routes.messages = [
      msg({ seq: 0, kind: "assistant", content: "改了 /tmp/proj/src/app.rs 请看" }),
    ];
    routes.files = [fileEntry("/tmp/proj/src/app.rs")];
    routes.fileContent = "fn main() {}";
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    fireEvent.click(await screen.findByTestId("file-link"));
    // 打开默认全屏 → 切横向分屏（左对话右文件）
    fireEvent.click(await screen.findByTestId("preview-toggle-split-h"));
    const preview = screen.getByTestId("file-preview");
    expect(preview.getAttribute("data-mode")).toBe("split-h");
    const split = screen.getByTestId("split-container");
    // 横向分屏容器：flex-row（左对话右文件）——与纵向 split 的 flex-col 区分
    expect(split.className).toContain("flex-row");
    // 对话与文件都在同一屏（同一容器内两个子区）；文件内容为异步拉取，等就绪
    expect(split.textContent).toContain("改了");
    expect((await screen.findByTestId("preview-code")).textContent).toContain("fn main() {}");
    // 三态互通：纵分屏 ↔ 横分屏 ↔ 全屏
    fireEvent.click(screen.getByTestId("preview-toggle-split"));
    expect(screen.getByTestId("file-preview").getAttribute("data-mode")).toBe("split");
    expect(screen.getByTestId("split-container").className).toContain("flex-col");
    fireEvent.click(screen.getByTestId("preview-toggle-split-h"));
    expect(screen.getByTestId("file-preview").getAttribute("data-mode")).toBe("split-h");
    fireEvent.click(screen.getByTestId("preview-toggle-fullscreen"));
    expect(screen.getByTestId("file-preview").getAttribute("data-mode")).toBe("fullscreen");
    expect(screen.queryByTestId("split-container")).toBeNull();
  });

  it("图标语义不得颠倒：split 按钮画上下两格（rows-2）、split-h 画左右两格（columns-2）", async () => {
    installFetch();
    routes.messages = [
      msg({ seq: 0, kind: "assistant", content: "改了 /tmp/proj/src/app.rs 请看" }),
    ];
    routes.files = [fileEntry("/tmp/proj/src/app.rs")];
    routes.fileContent = "fn main() {}";
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    fireEvent.click(await screen.findByTestId("file-link"));
    const btn = (id: string) => screen.getByTestId(id);
    // SVG 的 className 是 SVGAnimatedString，取 class 属性字符串
    const iconClass = (id: string) => btn(id).querySelector("svg")!.getAttribute("class") ?? "";
    // 上下分屏（split）= 上下两格 → rows-2
    expect(iconClass("preview-toggle-split")).toContain("lucide-rows-2");
    // 左右分屏（split-h）= 左右两格 → columns-2
    expect(iconClass("preview-toggle-split-h")).toContain("lucide-columns-2");
  });

  // 分屏态对话能力（2026-09-19 用户裁决）：分屏 = 对话列 + 文件列的并列布局，
  // 对话列必须保有完整对话能力。原实现把 MessageComposer / ApproveCard 排除在
  // 分屏分支外（仅非分屏正文视图挂载），致分屏看文件时**输入框消失、红卡不可见**——
  // 用户实测报告，且该行为在 docs/ 全库无任何设计依据（系实现越权）。
  // 本组为用户可见行为的回归锁：分屏两态（split / split-h）下两者都必须挂载。
  describe("分屏态对话能力（2026-09-19 用户裁决回归锁）", () => {
    it("上下分屏（split）下发送输入框仍挂载——分屏看文件也能发消息", async () => {
      installFetch();
      routes.messages = [
        msg({ seq: 0, kind: "assistant", content: "改了 /tmp/proj/src/app.rs 请看" }),
      ];
      routes.files = [fileEntry("/tmp/proj/src/app.rs")];
      routes.fileContent = "fn main() {}";
      render(<SessionDetail session={makeSession()} onBack={() => {}} />);
      fireEvent.click(await screen.findByTestId("file-link"));
      fireEvent.click(await screen.findByTestId("preview-toggle-split"));
      const split = screen.getByTestId("split-container");
      // 关键断言：输入框**在分屏容器内**（修正前分屏分支不挂 composer；
      // 必须用包含关系锁死，避免被其他分支误命中）
      expect(await within(split).findByTestId("message-composer")).toBeTruthy();
    });

    it("左右分屏（split-h）下发送输入框仍挂载", async () => {
      installFetch();
      routes.messages = [
        msg({ seq: 0, kind: "assistant", content: "改了 /tmp/proj/src/app.rs 请看" }),
      ];
      routes.files = [fileEntry("/tmp/proj/src/app.rs")];
      routes.fileContent = "fn main() {}";
      render(<SessionDetail session={makeSession()} onBack={() => {}} />);
      fireEvent.click(await screen.findByTestId("file-link"));
      fireEvent.click(await screen.findByTestId("preview-toggle-split-h"));
      const split = screen.getByTestId("split-container");
      expect(await within(split).findByTestId("message-composer")).toBeTruthy();
    });

    it("waiting 态上下分屏下审批红卡仍挂载——分屏也必须能看到待审批", async () => {
      installFetch();
      // 消息正文须含项目文件路径才会渲染 file-link（openFile 入口）
      routes.messages = [
        msg({ seq: 0, kind: "assistant", content: "等批准，相关文件 /tmp/proj/src/app.rs" }),
      ];
      routes.files = [fileEntry("/tmp/proj/src/app.rs")];
      routes.fileContent = "fn main() {}";
      routes.approveOptions = {
        available: true,
        options: [
          { id: "allow", label: "允许" },
          { id: "deny", label: "拒绝" },
        ],
        verifiedWith: "claude 2.1.251",
        currentVersion: "claude 2.1.251",
        drift: false,
      };
      render(<SessionDetail session={makeSession({ status: "waiting" })} onBack={() => {}} />);
      fireEvent.click(await screen.findByTestId("file-link"));
      fireEvent.click(await screen.findByTestId("preview-toggle-split"));
      const split = screen.getByTestId("split-container");
      expect(split).toBeTruthy();
      // 关键断言：红卡**在分屏容器内**（修正前分屏分支不挂红卡；仅断言
      // findByTestId 会被非分屏分支或浮层误命中 → 必须用包含关系锁死）
      expect(within(split).getByTestId("approve-card")).toBeTruthy();
    });

    it("waiting 态左右分屏下审批红卡仍挂载", async () => {
      installFetch();
      routes.messages = [
        msg({ seq: 0, kind: "assistant", content: "等批准，相关文件 /tmp/proj/src/app.rs" }),
      ];
      routes.files = [fileEntry("/tmp/proj/src/app.rs")];
      routes.fileContent = "fn main() {}";
      routes.approveOptions = {
        available: true,
        options: [
          { id: "allow", label: "允许" },
          { id: "deny", label: "拒绝" },
        ],
        verifiedWith: "claude 2.1.251",
        currentVersion: "claude 2.1.251",
        drift: false,
      };
      render(<SessionDetail session={makeSession({ status: "waiting" })} onBack={() => {}} />);
      fireEvent.click(await screen.findByTestId("file-link"));
      fireEvent.click(await screen.findByTestId("preview-toggle-split-h"));
      const split = screen.getByTestId("split-container");
      expect(within(split).getByTestId("approve-card")).toBeTruthy();
    });

    it("非 waiting 态分屏下不渲染红卡（状态门不变）", async () => {
      installFetch();
      routes.messages = [
        msg({ seq: 0, kind: "assistant", content: "改了 /tmp/proj/src/app.rs 请看" }),
      ];
      routes.files = [fileEntry("/tmp/proj/src/app.rs")];
      routes.fileContent = "fn main() {}";
      render(<SessionDetail session={makeSession({ status: "idle" })} onBack={() => {}} />);
      fireEvent.click(await screen.findByTestId("file-link"));
      fireEvent.click(await screen.findByTestId("preview-toggle-split"));
      expect(screen.getByTestId("split-container")).toBeTruthy();
      expect(screen.queryByTestId("approve-card")).toBeNull();
      // 但输入框仍在（两者门控条件不同：红卡看状态，输入框无条件）
      expect(await screen.findByTestId("message-composer")).toBeTruthy();
    });

    it("全屏浮层态：红卡与输入框不挂载（浮层覆盖对话属预期，非本裁决范围）", async () => {
      installFetch();
      routes.messages = [
        msg({ seq: 0, kind: "assistant", content: "改了 /tmp/proj/src/app.rs 请看" }),
      ];
      routes.files = [fileEntry("/tmp/proj/src/app.rs")];
      routes.fileContent = "fn main() {}";
      render(<SessionDetail session={makeSession()} onBack={() => {}} />);
      fireEvent.click(await screen.findByTestId("file-link"));
      // file-link 打开默认即全屏
      expect(screen.getByTestId("file-preview").getAttribute("data-mode")).toBe("fullscreen");
      expect(screen.queryByTestId("split-container")).toBeNull();
    });
  });

  it("需求：分屏分隔条可拖动——横向拖动改变文件栏宽度，纵向拖动改变高度", async () => {
    installFetch();
    routes.messages = [
      msg({ seq: 0, kind: "assistant", content: "改了 /tmp/proj/src/app.rs 请看" }),
    ];
    routes.files = [fileEntry("/tmp/proj/src/app.rs")];
    routes.fileContent = "fn main() {}";
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    fireEvent.click(await screen.findByTestId("file-link"));

    // ---- 横向分屏：拖分隔条 → 文件栏宽度变化（百分比） ----
    fireEvent.click(await screen.findByTestId("preview-toggle-split-h"));
    const container = screen.getByTestId("split-container");
    // jsdom 无布局引擎：注入容器矩形（宽 400）使拖动换算可算
    container.getBoundingClientRect = () =>
      ({ left: 0, top: 0, width: 400, height: 600, right: 400, bottom: 600 }) as DOMRect;
    const handleH = screen.getByTestId("split-handle");
    const filePane = screen.getByTestId("split-file-pane");
    const initial = parseFloat(filePane.style.width);
    fireEvent.pointerDown(handleH, { clientX: 300 });
    fireEvent.pointerMove(window, { clientX: 320 });
    fireEvent.pointerUp(window);
    const narrower = parseFloat(filePane.style.width);
    // 向右拖 20px / 容器宽 400 → 分隔条右移 → 右侧文件栏变窄 5 个百分点
    // （2026-09-16 用户裁决：分隔条跟随指针）
    expect(narrower).toBeCloseTo(initial - 5, 1);
    // 反向：向左拖回（文件栏变宽）
    fireEvent.pointerDown(handleH, { clientX: 320 });
    fireEvent.pointerMove(window, { clientX: 260 });
    fireEvent.pointerUp(window);
    expect(parseFloat(filePane.style.width)).toBeCloseTo(narrower + 15, 1);

    // ---- 纵向分屏：拖分隔条 → 文件栏高度变化 ----
    // 2026-09-20 用户裁决：竖屏 split 换位为文件在上、对话在下（ratioPane="before"）——
    // 文件栏在分隔条**上方**，拖动方向随之取反：向下拖 = 分隔条下移把上方文件栏撑大。
    // 「分隔条跟随指针」裁决不变（拖哪边文件栏都变小）
    fireEvent.click(screen.getByTestId("preview-toggle-split"));
    const containerV = screen.getByTestId("split-container");
    containerV.getBoundingClientRect = () =>
      ({ left: 0, top: 0, width: 400, height: 800, right: 400, bottom: 800 }) as DOMRect;
    const handleV = screen.getByTestId("split-handle");
    const filePaneV = screen.getByTestId("split-file-pane");
    const h0 = parseFloat(filePaneV.style.height);
    fireEvent.pointerDown(handleV, { clientY: 400 });
    fireEvent.pointerMove(window, { clientY: 500 });
    fireEvent.pointerUp(window);
    // 向下拖 100px / 容器高 800 → 分隔条下移 → 上方文件栏变高 12.5 个百分点
    expect(parseFloat(filePaneV.style.height)).toBeCloseTo(h0 + 12.5, 1);
    // 反向：向上拖回（上方文件栏变矮）
    fireEvent.pointerDown(handleV, { clientY: 500 });
    fireEvent.pointerMove(window, { clientY: 400 });
    fireEvent.pointerUp(window);
    expect(parseFloat(filePaneV.style.height)).toBeCloseTo(h0, 1);

    // ---- 拖动不得越界（钳制 15%–85%） ----
    fireEvent.pointerDown(handleV, { clientY: 0 });
    fireEvent.pointerMove(window, { clientY: -100000 });
    fireEvent.pointerUp(window);
    expect(parseFloat(filePaneV.style.height)).toBeGreaterThanOrEqual(14.9);
    fireEvent.pointerDown(handleV, { clientY: 800 });
    fireEvent.pointerMove(window, { clientY: 100000 });
    fireEvent.pointerUp(window);
    expect(parseFloat(filePaneV.style.height)).toBeLessThanOrEqual(85.1);
  });

  // 竖屏分屏换位（2026-09-20 用户裁决）：split 视觉顺序 = 文件在上、对话在下
  // （输入框贴底）；split-h 维持对话在左、文件在右。CSS order 视觉换位，
  // DOM 顺序两态一致（对话优先，a11y 不变）
  it("竖屏 split 视觉换位：文件 order-1 在上、对话 order-3 在下；split-h 无 order", async () => {
    installFetch();
    routes.messages = [
      msg({ seq: 0, kind: "assistant", content: "改了 /tmp/proj/src/app.rs 请看" }),
    ];
    routes.files = [fileEntry("/tmp/proj/src/app.rs")];
    routes.fileContent = "fn main() {}";
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    fireEvent.click(await screen.findByTestId("file-link"));
    fireEvent.click(await screen.findByTestId("preview-toggle-split"));
    const container = screen.getByTestId("split-container");
    const convCol = container.querySelector(".flex.min-h-0.min-w-0.flex-1.flex-col")!;
    const filePane = screen.getByTestId("split-file-pane");
    const handle = screen.getByTestId("split-handle");
    expect(convCol.className).toContain("order-3");
    expect(filePane.className).toContain("order-1");
    expect(handle.className).toContain("order-2");
    // 最小高度保护（仅 split）：对话列有 minHeight，文件栏有 maxHeight 上限
    expect(convCol.getAttribute("style")).toContain("min-height");
    expect(filePane.getAttribute("style")).toContain("max-height");

    // 切横向分屏：两态语义各自独立——无 order 类、无高度保护
    fireEvent.click(await screen.findByTestId("preview-toggle-split-h"));
    const containerH = screen.getByTestId("split-container");
    const convColH = containerH.querySelector(".flex.min-h-0.min-w-0.flex-1.flex-col")!;
    const filePaneH = screen.getByTestId("split-file-pane");
    expect(convColH.className).not.toContain("order-");
    expect(filePaneH.className).not.toContain("order-");
    expect(convColH.getAttribute("style")).not.toContain("min-height");
    expect(filePaneH.getAttribute("style")).not.toContain("max-height");
  });

  it("切换器只保留预览页头一份（2026-09-16 裁决），不占详情页头空间", async () => {
    installFetch();
    routes.messages = [
      msg({ seq: 0, kind: "assistant", content: "改了 /tmp/proj/src/app.rs 请看" }),
    ];
    routes.files = [fileEntry("/tmp/proj/src/app.rs")];
    routes.fileContent = "fn main() {}";
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);

    // 预览未打开：整个页面没有任何布局切换器
    expect(screen.queryAllByTestId("preview-toggle-split")).toHaveLength(0);
    expect(screen.queryAllByTestId("preview-mode-split")).toHaveLength(0);

    fireEvent.click(await screen.findByTestId("file-link"));
    // 打开后：切换器恰好一份（旧版两份——详情页头 + 预览页头），且属于预览页头
    expect(screen.getAllByTestId("preview-toggle-split")).toHaveLength(1);
    // 详情页头那份（旧 testid 前缀 preview-mode-*）不得再存在
    expect(screen.queryAllByTestId("preview-mode-split")).toHaveLength(0);
    expect(screen.queryAllByTestId("preview-mode-split-h")).toHaveLength(0);
    expect(screen.queryAllByTestId("preview-mode-fullscreen")).toHaveLength(0);
    // 唯一那份在 file-preview 容器内（预览页头）
    const previewEl = screen.getByTestId("file-preview");
    expect(previewEl.contains(screen.getByTestId("preview-toggle-split"))).toBe(true);
    expect(previewEl.contains(screen.getByTestId("preview-toggle-split-h"))).toBe(true);
    expect(previewEl.contains(screen.getByTestId("preview-toggle-fullscreen"))).toBe(true);

    // 三态在唯一入口下仍全通：全屏 → 左右 → 上下 → 回全屏
    fireEvent.click(screen.getByTestId("preview-toggle-split-h"));
    expect(screen.getByTestId("file-preview").getAttribute("data-mode")).toBe("split-h");
    // 分屏态（文件区顶部页头）同样只有一份
    expect(screen.getAllByTestId("preview-toggle-split")).toHaveLength(1);
    fireEvent.click(screen.getByTestId("preview-toggle-split"));
    expect(screen.getByTestId("file-preview").getAttribute("data-mode")).toBe("split");
    fireEvent.click(screen.getByTestId("preview-toggle-fullscreen"));
    expect(screen.getByTestId("file-preview").getAttribute("data-mode")).toBe("fullscreen");
  });

  it("Task 2：页头恒有文件面板入口，点击进入列表视图（窄屏默认全屏布局）", async () => {
    installFetch();
    routes.messages = [
      msg({ seq: 0, kind: "assistant", content: "改了 /tmp/proj/src/app.rs 请看" }),
    ];
    routes.files = [fileEntry("/tmp/proj/src/app.rs")];
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    // 页头按钮恒可见（预览未打开时也有）
    const btn = await screen.findByTestId("file-panel-button");
    expect(btn.getAttribute("aria-label")).toBe("文件面板");
    // 预览未打开：无侧栏容器
    expect(screen.queryByTestId("preview-shell")).toBeNull();
    fireEvent.click(btn);
    // 进入列表视图：侧栏容器出现（jsdom 无 matchMedia → 窄屏默认 fullscreen）
    const shell = await screen.findByTestId("preview-shell");
    expect(shell.getAttribute("data-view")).toBe("list");
    expect(shell.getAttribute("data-mode")).toBe("fullscreen");
  });

  it("Task 2：宽屏打开面板默认 split-h（左对话右列表）", async () => {
    installFetch();
    routes.messages = [msg({ seq: 0, kind: "assistant", content: "hi" })];
    routes.files = [fileEntry("/tmp/proj/src/app.rs")];
    Object.defineProperty(window, "matchMedia", {
      writable: true,
      value: (q: string) => ({ matches: q.includes("min-width"), media: q }),
    });
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    fireEvent.click(await screen.findByTestId("file-panel-button"));
    const shell = await screen.findByTestId("preview-shell");
    expect(shell.getAttribute("data-mode")).toBe("split-h");
    expect(screen.getByTestId("split-container").className).toContain("flex-row");
  });

  it("Task 2：面板挂载复用详情页已拉的提取结果，切档才重拉（scope→limit）", async () => {
    installFetch();
    routes.messages = [msg({ seq: 0, kind: "assistant", content: "改了 /tmp/proj/src/app.rs" })];
    routes.files = [fileEntry("/tmp/proj/src/app.rs")];
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    // 挂载时已拉一次（scope 默认 200，用于正文链接化）
    await screen.findByTestId("file-panel-button");
    await waitFor(() => {
      expect(fetchMock.mock.calls.some((c: unknown[]) => String(c[0]).includes("limit=200"))).toBe(
        true
      );
    });
    const before = fetchMock.mock.calls.filter((c: unknown[]) =>
      String(c[0]).includes("/session-files")
    ).length;
    // 打开面板不重拉（复用挂载结果）
    fireEvent.click(screen.getByTestId("file-panel-button"));
    await screen.findByTestId("preview-shell");
    expect(
      fetchMock.mock.calls.filter((c: unknown[]) => String(c[0]).includes("/session-files")).length
    ).toBe(before);
  }, 15000);

  it("Task 3 集成：面板 → 点文件 → 预览带返回按钮 → 返回列表（布局保持）", async () => {
    installFetch();
    routes.messages = [msg({ seq: 0, kind: "assistant", content: "hi" })];
    // 后端契约：已按 lastSeq 降序（app.rs seq 9 在前，doc.md seq 5 在后）
    routes.files = [
      fileEntry("/tmp/proj/src/app.rs", { lastSeq: 9 }),
      fileEntry("/p/doc.md", { lastSeq: 5 }),
    ];
    routes.fileContent = "fn main() {}";
    routes.fileMime = "text/rust";
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    // 打开面板 → 列表视图（窄屏全屏）
    fireEvent.click(await screen.findByTestId("file-panel-button"));
    const shell = await screen.findByTestId("preview-shell");
    expect(shell.getAttribute("data-view")).toBe("list");
    // 面板列表渲染（后端顺序）
    expect(screen.getByTestId("file-row-0-open").textContent).toContain("app.rs");
    expect(screen.getByTestId("file-row-1-open").textContent).toContain("doc.md");
    // 点文件名 → 单文件预览 + 返回按钮
    fireEvent.click(screen.getByTestId("file-row-0-open"));
    expect((await screen.findByTestId("preview-shell")).getAttribute("data-view")).toBe("file");
    expect((await screen.findByTestId("preview-code")).textContent).toContain("fn main() {}");
    // 从面板进入 → 有返回按钮
    const backBtn = screen.getByTestId("preview-back-list");
    expect(backBtn.getAttribute("aria-label")).toBe("返回文件列表");
    // 返回列表：视图切回 list，布局 mode 保持
    fireEvent.click(backBtn);
    const back = screen.getByTestId("preview-shell");
    expect(back.getAttribute("data-view")).toBe("list");
    expect(back.getAttribute("data-mode")).toBe("fullscreen");
  }, 15000);

  it("Task 3：从消息正文链接进入预览无返回按钮（来源区分）", async () => {
    installFetch();
    routes.messages = [
      msg({ seq: 0, kind: "assistant", content: "改了 /tmp/proj/src/app.rs 请看" }),
    ];
    routes.files = [fileEntry("/tmp/proj/src/app.rs")];
    routes.fileContent = "fn main() {}";
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    fireEvent.click(await screen.findByTestId("file-link"));
    await screen.findByTestId("file-preview");
    expect(screen.queryByTestId("preview-back-list")).toBeNull();
  });

  it("ZCode 式面板按钮：开启态高亮 + tooltip，再点收回面板", async () => {
    installFetch();
    routes.messages = [msg({ seq: 0, kind: "assistant", content: "hi" })];
    routes.files = [fileEntry("/tmp/proj/src/app.rs")];
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    const btn = await screen.findByTestId("file-panel-button");
    // 未开启：无高亮（aria-pressed=false），tooltip 提示打开
    expect(btn.getAttribute("aria-pressed")).toBe("false");
    expect(btn.getAttribute("title")).toBe("打开文件面板");
    fireEvent.click(btn);
    // 开启：高亮（aria-pressed=true）+ tooltip 变「收起面板」
    expect(await screen.findByTestId("preview-shell"));
    expect(btn.getAttribute("aria-pressed")).toBe("true");
    expect(btn.getAttribute("title")).toBe("收起文件面板");
    // 再点：收回面板（回正文视图）
    fireEvent.click(btn);
    expect(screen.queryByTestId("preview-shell")).toBeNull();
    expect(btn.getAttribute("aria-pressed")).toBe("false");
  });

  it("字号档位（2026-09-16 用户裁决）：面板按钮旁选择，50/75/100/125 四档，主对话窗口生效", async () => {
    installFetch();
    routes.messages = [msg({ seq: 0, kind: "assistant", content: "正文" })];
    routes.files = [fileEntry("/tmp/proj/src/app.rs")];
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    await screen.findByText("正文");
    const sel = (await screen.findByTestId("font-scale-select")) as HTMLSelectElement;
    // 四档可选，默认 100
    expect([...sel.options].map((o) => o.value)).toEqual(["0.5", "0.75", "1", "1.25"]);
    expect(sel.value).toBe("1");
    // 切到 125% → 消息区容器带档位类（字号经 CSS 变量缩放）
    fireEvent.change(sel, { target: { value: "1.25" } });
    expect(screen.getByTestId("message-area").getAttribute("data-font-scale")).toBe("1.25");
    // 切到 50%
    fireEvent.change(sel, { target: { value: "0.5" } });
    expect(screen.getByTestId("message-area").getAttribute("data-font-scale")).toBe("0.5");
  });

  it("进入详情默认滚到底部（最新消息）；加载更早按钮在消息列表最上方", async () => {
    installFetch();
    routes.messages = Array.from({ length: 200 }, (_, i) =>
      msg({ seq: i, kind: "user", content: `m${i}` })
    );
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    await screen.findByText("m199");

    // 进入即滚到底：jsdom 无布局引擎（scrollHeight 恒 0），注入有限值后
    // 触发一次手动刷新（等价于「数据到达」）让滚动 effect 有可观测量
    const area = screen.getByTestId("message-area");
    Object.defineProperty(area, "scrollHeight", { value: 5000, configurable: true });
    fireEvent.click(screen.getByTestId("detail-refresh"));
    await waitFor(() => expect(area.scrollTop).toBe(5000));

    // 「加载更早消息」应排在消息列表**之前**（用户往上翻到头才点它）
    const list = area.querySelector("ul");
    const loadMore = screen.getByTestId("load-more");
    expect(list).toBeTruthy();
    expect(list!.compareDocumentPosition(loadMore) & Node.DOCUMENT_POSITION_PRECEDING).toBeTruthy();
  }, 15000);

  it("点加载更早后保持阅读位置（顶部插入量补偿，视线不跳）", async () => {
    installFetch();
    routes.messages = Array.from({ length: 200 }, (_, i) =>
      msg({ seq: i, kind: "user", content: `m${i}` })
    );
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    await screen.findByText("m199");
    const area = screen.getByTestId("message-area");
    // 模拟：当前滚动位置 1000，内容总高 5000
    Object.defineProperty(area, "scrollHeight", { value: 5000, configurable: true });
    area.scrollTop = 1000;
    // 点「加载更早」→ 记录锚点；limit=400 重拉在途。先挂起响应、注入新内容总高
    // （8000）后放行——根治顺序竞态（旧版靠 detail-refresh 二次触发对齐断言瞬时值：
    // 若首响落在几何量注入前，锚被 0 插入量消费，二次落底把 scrollTop 盖写为
    // 8000，机器负载下偶发翻车）。
    const inner = fetchMock;
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const gated = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/session-messages") && url.includes("limit=400")) {
        await gate; // 挂起重拉响应，等几何量注入
      }
      return inner(input);
    });
    vi.stubGlobal("fetch", gated);
    fireEvent.click(screen.getByTestId("load-more"));
    await waitFor(() => {
      expect(gated.mock.calls.some((c: unknown[]) => String(c[0]).includes("limit=400"))).toBe(
        true
      );
    });
    Object.defineProperty(area, "scrollHeight", { value: 8000, configurable: true });
    release(); // 放行 limit=400 响应 → 数据落地 → 补偿对齐
    await waitFor(() => {
      // 补偿：1000 + (8000 - 5000) = 4000（视线停在原内容处）
      expect(area.scrollTop).toBe(4000);
    });
  }, 15000);

  it("书签集成：打标签 → 消息旁角标 → 点色点跳转（scrollIntoView 落在目标）", async () => {
    installFetch();
    routes.messages = [
      msg({ seq: 0, kind: "user", content: "第一条指令" }),
      msg({ seq: 1, kind: "assistant", content: "第一段回复" }),
      msg({ seq: 2, kind: "user", content: "待会回来看这条" }),
      msg({ seq: 3, kind: "assistant", content: "最后一段" }),
    ];
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    await screen.findByText("最后一段");

    const area = screen.getByTestId("message-area");
    // jsdom 无布局：注入几何量——容器顶边 100，seq 2 的底边 200（= 视口首条）
    area.getBoundingClientRect = () =>
      ({ top: 100, bottom: 700, left: 0, right: 400, width: 400, height: 600 }) as DOMRect;
    const liOf = (seq: number) => screen.getByTestId(`msg-${seq}`);
    // 视口顶边 = 100：seq 0/1 已完全滚出上方（bottom ≤ 100），seq 2 是首条可见
    liOf(0).getBoundingClientRect = () =>
      ({ top: -60, bottom: -10, left: 0, right: 400, width: 400, height: 50 }) as DOMRect;
    liOf(1).getBoundingClientRect = () =>
      ({ top: 20, bottom: 90, left: 0, right: 400, width: 400, height: 70 }) as DOMRect;
    liOf(2).getBoundingClientRect = () =>
      ({ top: 110, bottom: 260, left: 0, right: 400, width: 400, height: 150 }) as DOMRect;
    liOf(3).getBoundingClientRect = () =>
      ({ top: 270, bottom: 400, left: 0, right: 400, width: 400, height: 130 }) as DOMRect;

    // 打标签：点 + → 选第一个颜色 → 落在视口首条（seq 2「待会回来看这条」）
    fireEvent.click(screen.getByTestId("bookmark-add"));
    fireEvent.click(screen.getByTestId(`bookmark-color-${BOOKMARK_COLORS[0]}`));
    // 色点出现 + 目标消息旁有角标
    expect(screen.getByTestId(`bookmark-dot-${BOOKMARK_COLORS[0]}`)).toBeTruthy();
    expect(screen.getByTestId("msg-bookmark-2")).toBeTruthy();
    // 角标只落在命中那条
    expect(screen.queryByTestId("msg-bookmark-0")).toBeNull();

    // 点色点跳转：scrollIntoView 落在 seq 2 的 li 上
    const target = liOf(2);
    const scrollSpy = vi.fn();
    target.scrollIntoView = scrollSpy;
    fireEvent.click(screen.getByTestId(`bookmark-dot-${BOOKMARK_COLORS[0]}`));
    expect(scrollSpy).toHaveBeenCalled();
    expect(scrollSpy.mock.calls[0][0]).toMatchObject({ block: "start" });
  }, 15000);

  it("书签集成：删除单条与清空全部", async () => {
    installFetch();
    routes.messages = [
      msg({ seq: 0, kind: "user", content: "甲" }),
      msg({ seq: 1, kind: "user", content: "乙" }),
    ];
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    await screen.findByText("乙");
    const area = screen.getByTestId("message-area");
    area.getBoundingClientRect = () =>
      ({ top: 0, bottom: 600, left: 0, right: 400, width: 400, height: 600 }) as DOMRect;
    screen.getByTestId("msg-0").getBoundingClientRect = () =>
      ({ top: 0, bottom: 50, left: 0, right: 400, width: 400, height: 50 }) as DOMRect;
    screen.getByTestId("msg-1").getBoundingClientRect = () =>
      ({ top: 50, bottom: 100, left: 0, right: 400, width: 400, height: 100 }) as DOMRect;

    // 打两个不同颜色的标签
    fireEvent.click(screen.getByTestId("bookmark-add"));
    fireEvent.click(screen.getByTestId(`bookmark-color-${BOOKMARK_COLORS[0]}`));
    fireEvent.click(screen.getByTestId("bookmark-add"));
    fireEvent.click(screen.getByTestId(`bookmark-color-${BOOKMARK_COLORS[1]}`));
    expect(screen.getAllByTestId(/^bookmark-dot-/)).toHaveLength(2);

    // 管理态删单条
    fireEvent.click(screen.getByTestId("bookmark-manage"));
    fireEvent.click(screen.getByTestId(`bookmark-remove-${BOOKMARK_COLORS[0]}`));
    expect(screen.getAllByTestId(/^bookmark-(dot|remove)-/)).toHaveLength(1);
    // 清空全部
    fireEvent.click(screen.getByTestId("bookmark-clear-all"));
    expect(screen.queryAllByTestId(/^bookmark-(dot|remove)-/)).toHaveLength(0);
    // 角标也一并消失
    expect(screen.queryByTestId("msg-bookmark-0")).toBeNull();
  }, 15000);

  it("书签集成：返回看板再进同一会话，书签仍在（store 跨卸载恢复）", async () => {
    installFetch();
    routes.messages = [msg({ seq: 0, kind: "user", content: "记住我" })];
    const { unmount } = render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    await screen.findByText("记住我");
    const area = screen.getByTestId("message-area");
    area.getBoundingClientRect = () =>
      ({ top: 0, bottom: 600, left: 0, right: 400, width: 400, height: 600 }) as DOMRect;
    screen.getByTestId("msg-0").getBoundingClientRect = () =>
      ({ top: 0, bottom: 50, left: 0, right: 400, width: 400, height: 50 }) as DOMRect;
    fireEvent.click(screen.getByTestId("bookmark-add"));
    fireEvent.click(screen.getByTestId(`bookmark-color-${BOOKMARK_COLORS[3]}`));
    expect(screen.getByTestId(`bookmark-dot-${BOOKMARK_COLORS[3]}`)).toBeTruthy();

    // 「返回看板」= 卸载本组件（App.tsx 条件挂载）
    unmount();
    // 再进同一会话（同 session.id）→ 书签从模块级 store 恢复
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    await screen.findByText("记住我");
    expect(screen.getByTestId(`bookmark-dot-${BOOKMARK_COLORS[3]}`)).toBeTruthy();
    expect(screen.getByTestId("msg-bookmark-0")).toBeTruthy();
  }, 15000);

  it("书签集成：刷新页面（同 bootId 重挂）书签从 localStorage 恢复", async () => {
    installFetch();
    routes.messages = [msg({ seq: 0, kind: "user", content: "记住我" })];
    // 第一次「打开页面」：打书签（写入 localStorage 镜像）
    const { unmount } = render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    await screen.findByText("记住我");
    const area = screen.getByTestId("message-area");
    area.getBoundingClientRect = () =>
      ({ top: 0, bottom: 600, left: 0, right: 400, width: 400, height: 600 }) as DOMRect;
    screen.getByTestId("msg-0").getBoundingClientRect = () =>
      ({ top: 0, bottom: 50, left: 0, right: 400, width: 400, height: 50 }) as DOMRect;
    fireEvent.click(screen.getByTestId("bookmark-add"));
    fireEvent.click(screen.getByTestId(`bookmark-color-${BOOKMARK_COLORS[3]}`));
    expect(window.localStorage.getItem("mam-bookmarks")).toContain(BOOKMARK_COLORS[3]);
    unmount();

    // 「刷新页面」：重挂（bootId 不变，内存单例从 localStorage 种回）
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    await screen.findByText("记住我");
    expect(await screen.findByTestId(`bookmark-dot-${BOOKMARK_COLORS[3]}`)).toBeTruthy();
    expect(screen.getByTestId("msg-bookmark-0")).toBeTruthy();
  }, 15000);

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
    expect((await screen.findByTestId("detail-error")).textContent).toContain("无法读取该会话内容");
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

describe("书签跨加载窗口跳转（M5 P3-c）", () => {
  /** 250 条合成消息（seq = 下标）；书签目标 seq=10 在首批 200 条窗口之外 */
  function bigMessages(): SessionMessage[] {
    return Array.from({ length: 250 }, (_, i) =>
      msg({
        seq: i,
        kind: i === 10 ? "user" : i % 2 ? "assistant" : "user",
        content: i === 10 ? "书签目标：这条在很早的分页里" : `填充消息 ${i}`,
        ts: 1000 + i,
      })
    );
  }

  function installLimitAwareFetch(all: SessionMessage[]) {
    fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/session-messages")) {
        const limit = Number(new URL(url, "http://x").searchParams.get("limit") ?? 200);
        return new Response(JSON.stringify({ messages: all.slice(-limit), truncated: true }), {
          status: 200,
        });
      }
      if (url.includes("/host")) {
        return new Response(
          JSON.stringify({
            host: { name: "n", platform: "windows", version: "0", bootId: "boot-test" },
            enabledTools: [],
          }),
          { status: 200 }
        );
      }
      if (url.includes("/session-files")) {
        return new Response(JSON.stringify({ files: [], truncated: false }), { status: 200 });
      }
      throw new Error(`unexpected fetch: ${url}`);
    });
    vi.stubGlobal("fetch", fetchMock);
  }

  function seedBookmarkFor(m: SessionMessage) {
    window.localStorage.setItem(
      "mam-bookmarks",
      JSON.stringify({
        bootId: "boot-test",
        sessions: {
          "sess-1": [
            {
              color: BOOKMARK_COLORS[1],
              seq: m.seq,
              anchor: messageAnchor(m),
              preview: m.content.slice(0, 40),
            },
          ],
        },
      })
    );
  }

  it("书签目标在首批窗口之外 → 点击自动逐级加载更早直至命中滚动", async () => {
    const all = bigMessages();
    const target = all[10];
    seedBookmarkFor(target);
    installLimitAwareFetch(all);

    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    // 首批窗口（200 条）：目标 seq 10 不在窗口
    await screen.findByText("填充消息 249");

    const scrollSpy = vi.fn();
    Element.prototype.scrollIntoView = scrollSpy;
    fireEvent.click(screen.getByTestId(`bookmark-dot-${BOOKMARK_COLORS[1]}`));

    // 自动扩窗：limit 200→400 重拉，目标（seq 10）出现并被滚动定位。
    // timeout 10s：与下例「扩窗到顶」同因——扩窗重拉在整库并行负载下可超 waitFor
    // 默认 1s（Task 8 门前实测整库跑两次假失败两次，单文件连跑 5/5 绿）；
    // 只放宽等待上限，断言语义不变
    await waitFor(
      () => {
        expect(fetchMock).toHaveBeenCalledWith(expect.stringContaining("limit=400"));
      },
      { timeout: 10_000 }
    );
    await screen.findByTestId("msg-10");
    await waitFor(() => expect(scrollSpy).toHaveBeenCalled(), { timeout: 10_000 });
    expect(scrollSpy.mock.calls[0][0]).toMatchObject({ block: "start" });
    // 定位成功：加载/miss 横幅均不在场
    expect(screen.queryByTestId("bookmark-jump-miss")).toBeNull();
    expect(screen.queryByTestId("bookmark-jump-loading")).toBeNull();
  }, 15000);

  it("扩窗到顶（MAX_LIMIT）仍未命中 → miss 横幅", async () => {
    const all = bigMessages();
    seedBookmarkFor({
      seq: 999,
      kind: "user",
      content: "这条书签指向不存在的消息",
      ts: 1234,
    });
    installLimitAwareFetch(all);

    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    await screen.findByText("填充消息 249");

    const scrollSpy = vi.fn();
    Element.prototype.scrollIntoView = scrollSpy;
    fireEvent.click(screen.getByTestId(`bookmark-dot-${BOOKMARK_COLORS[1]}`));

    // 逐级扩到 MAX_LIMIT（1000）仍未命中 → miss 横幅，且未发生任何滚动。
    // timeout 10s：四级扩窗（200→…→1000）在 CI 慢机上实测 >4s（本地快机 <1s），
    // 3s 曾在 CI 抖动失败（run 35314166316）
    await waitFor(() => expect(screen.getByTestId("bookmark-jump-miss")).toBeTruthy(), {
      timeout: 10_000,
    });
    expect(scrollSpy).not.toHaveBeenCalled();
  }, 15000);
});

// ==== F6：详情页 10s 轮询（假计时器锁节奏与可见性语义）====
describe("SessionDetail：详情页 10s 轮询（F6）", () => {
  /** 只数 /session-messages 调用（/host、/session-files 的拉取不计入节奏断言） */
  function messagesCalls(): number {
    return fetchMock.mock.calls.filter((c: unknown[]) => String(c[0]).includes("/session-messages"))
      .length;
  }

  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("节奏：挂载首拉 1 次，+10s 轮询第 2 次，再 +10s 第 3 次（首拉不双触发）", async () => {
    installFetch();
    routes.messages = [msg({ seq: 0, kind: "user", content: "首拉" })];
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    await act(async () => {}); // 挂载首拉落地（轮询 interval 首拍在 +10s，不立即触发）
    expect(messagesCalls()).toBe(1);
    await act(async () => {
      vi.advanceTimersByTime(10_000);
    });
    expect(messagesCalls()).toBe(2);
    await act(async () => {
      vi.advanceTimersByTime(10_000);
    });
    expect(messagesCalls()).toBe(3);
  });

  it("hidden 暂停：推进计时器不触发；恢复 visible 立即补刷一次再续 10s 节奏", async () => {
    installFetch();
    routes.messages = [msg({ seq: 0, kind: "user", content: "首拉" })];
    const visSpy = vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    await act(async () => {});
    expect(messagesCalls()).toBe(1);
    // 切后台（hidden + visibilitychange）：暂停轮询——推进 30s 零新增拉取
    visSpy.mockReturnValue("hidden");
    await act(async () => {
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await act(async () => {
      vi.advanceTimersByTime(30_000);
    });
    expect(messagesCalls()).toBe(1);
    // 切回前台：立即补刷一次（追回隐藏期间错过的更新）
    visSpy.mockReturnValue("visible");
    await act(async () => {
      document.dispatchEvent(new Event("visibilitychange"));
    });
    expect(messagesCalls()).toBe(2);
    // 补刷后重启节奏：+10s 下一拍
    await act(async () => {
      vi.advanceTimersByTime(10_000);
    });
    expect(messagesCalls()).toBe(3);
    visSpy.mockRestore();
  });

  // 收尾批 P2：F6 卸载清理回归钉（评审修复批遗留的显式验证）——unmount 必须
  // 清 interval + 移除 visibilitychange 监听，长驻页面来回进出不泄漏计时器/监听。
  // 监听移除按 spyOn add/removeEventListener 捕获引用比对（同一函数引用注册且移除）；
  // interval 清理由 getTimerCount 直证（泄漏则卸载后仍挂 1 个待触发拍），并按
  // 「clearAllTimers 后再推进不再触发拉取」行为口径兜底断言
  it("unmount 清理：interval 已清 + visibilitychange 监听已移除", async () => {
    installFetch();
    routes.messages = [msg({ seq: 0, kind: "user", content: "首拉" })];
    const addSpy = vi.spyOn(document, "addEventListener");
    const removeSpy = vi.spyOn(document, "removeEventListener");
    try {
      const { unmount } = render(<SessionDetail session={makeSession()} onBack={() => {}} />);
      await act(async () => {}); // 挂载首拉落地
      expect(messagesCalls()).toBe(1);
      await act(async () => {
        vi.advanceTimersByTime(10_000);
      });
      expect(messagesCalls()).toBe(2); // 轮询确实在跑（前提自证，断言不空转）
      unmount();
      // visibilitychange 监听已移除：注册与移除是同一函数引用（组件只挂这一个
      // document 级监听——比对引用即精确钉住 F6 effect 的清理半边）
      const visListener = addSpy.mock.calls.find((c) => c[0] === "visibilitychange")?.[1];
      expect(visListener).toBeDefined();
      expect(removeSpy).toHaveBeenCalledWith("visibilitychange", visListener);
      // interval 已清：卸载后零待触发计时器（泄漏则此处为 1）
      expect(vi.getTimerCount()).toBe(0);
      // 行为口径兜底：清掉全部计时器再推进，不再触发任何拉取
      vi.clearAllTimers();
      await act(async () => {
        vi.advanceTimersByTime(60_000);
      });
      expect(messagesCalls()).toBe(2);
    } finally {
      addSpy.mockRestore();
      removeSpy.mockRestore();
    }
  });
});

// ==== P2-B（评审修复批）：轮询滚动跟随条件化 ====
// 轮询刷新数据落地时，仅在刷新前采样为「贴底」（距底 <120px）才跟随落底；
// 上翻阅读历史不被每 10s 拽回底部。假计时器 + 滚动容器几何量 mock
//（jsdom 无布局引擎：scrollHeight/clientHeight 逐实例注入，scrollTop 可赋可读）。
describe("SessionDetail：轮询滚动跟随条件化（P2-B）", () => {
  /** 只数 /session-messages 调用（证明刷新确实发生，断言不空转） */
  function messagesCalls(): number {
    return fetchMock.mock.calls.filter((c: unknown[]) => String(c[0]).includes("/session-messages"))
      .length;
  }

  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    // 假计时器必须还原（与文件内真实计时器用例共存，e39c3d9 自审提示）
    vi.useRealTimers();
  });

  /** 注入消息滚动容器几何量（元素挂载后逐实例 defineProperty，重渲染不丢） */
  function installGeometry(area: HTMLElement, geo: { scrollHeight: number; clientHeight: number }) {
    Object.defineProperty(area, "scrollHeight", { value: geo.scrollHeight, configurable: true });
    Object.defineProperty(area, "clientHeight", { value: geo.clientHeight, configurable: true });
  }

  it("poll_keeps_scroll_when_reading_history：上翻阅读（距底 ≥120px）两拍轮询刷新不拽回底部", async () => {
    installFetch();
    routes.messages = [msg({ seq: 0, kind: "user", content: "首拉" })];
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    await act(async () => {}); // 首拉落地（首拉无条件落底；此刻 scrollHeight=0 → scrollTop=0）
    const area = screen.getByTestId("message-area");
    // 距底 = 2000 - 0 - 500 = 1500 ≥ 120 → 非贴底（用户上翻阅读历史）
    installGeometry(area, { scrollHeight: 2000, clientHeight: 500 });
    area.scrollTop = 0;
    // 第一拍轮询：新消息落地（mock 按 routes 现取 → 新数组触发对齐 effect）
    routes.messages = [
      msg({ seq: 0, kind: "user", content: "首拉" }),
      msg({ seq: 1, kind: "assistant", content: "轮询新消息一" }),
    ];
    await act(async () => {
      vi.advanceTimersByTime(10_000);
    });
    expect(messagesCalls()).toBe(2); // 刷新确实发生
    expect(area.scrollTop).toBe(0); // 但滚动位置不动（距底 ≥120px 不跟随）
    // 第二拍轮询：仍不跟随
    routes.messages = [
      ...routes.messages,
      msg({ seq: 2, kind: "assistant", content: "轮询新消息二" }),
    ];
    await act(async () => {
      vi.advanceTimersByTime(10_000);
    });
    expect(messagesCalls()).toBe(3);
    expect(area.scrollTop).toBe(0);
  });

  it("poll_follows_when_near_bottom：贴底（距底 <120px）轮询刷新到新消息跟随落底", async () => {
    installFetch();
    routes.messages = [msg({ seq: 0, kind: "user", content: "首拉" })];
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    await act(async () => {});
    const area = screen.getByTestId("message-area");
    // 距底 = 2000 - 1400 - 500 = 100 < 120 → 贴底
    installGeometry(area, { scrollHeight: 2000, clientHeight: 500 });
    area.scrollTop = 1400;
    // 轮询拍新消息落地 → 跟随落底：scrollTop = scrollHeight = 2000
    routes.messages = [
      msg({ seq: 0, kind: "user", content: "首拉" }),
      msg({ seq: 1, kind: "assistant", content: "贴底时的新消息" }),
    ];
    await act(async () => {
      vi.advanceTimersByTime(10_000);
    });
    expect(messagesCalls()).toBe(2);
    expect(area.scrollTop).toBe(2000);
  });

  it("first load 仍无条件落底：首拉对齐不依赖贴底采样（既有行为的显式回归锁）", async () => {
    installFetch();
    routes.messages = [msg({ seq: 0, kind: "user", content: "首拉" })];
    // 挂载前在 Element 原型注入 scrollHeight（元素尚不存在，无法逐实例注入；
    // jsdom 将 scrollHeight 定义为 Element.prototype 自有 getter，jsdom 探明）：
    // 首拉对齐量即可观测量 = scrollHeight
    const scrollHeightSpy = vi
      .spyOn(Element.prototype, "scrollHeight", "get")
      .mockReturnValue(2000);
    try {
      render(<SessionDetail session={makeSession()} onBack={() => {}} />);
      await act(async () => {}); // 首拉落地
      const area = screen.getByTestId("message-area");
      // 首拉落底：scrollTop = scrollHeight = 2000（不采样、不受 120px 阈值约束）
      expect(area.scrollTop).toBe(2000);
    } finally {
      scrollHeightSpy.mockRestore();
    }
  });
});

// ==== 计划正文渲染（2026-09-20 用户实测：ExitPlanMode 整篇计划在详情页是 \n 字面量汤）====
// 根因：后端把工具输入原封透传为 JSON 串（字符串值换行全为 \n 转义），前端 <pre> 原样上屏。
// 修法：toolArgs 解析出非空字符串 plan 字段 → 该正文走 markdown 渲染；不看 toolName——
// zcode 的 ExitPlanMode 输入同为 {plan} 但 MAM 记录的是显示 title，按名字匹配会漏。
// 其他工具参数维持原样（用户裁决：不做通用美化）。
describe("SessionDetail：计划正文渲染（2026-09-20）", () => {
  function expandToolCall(seq: number) {
    fireEvent.click(screen.getByTestId(`msg-${seq}-toggle`));
  }

  it("ExitPlanMode：plan 字段按 markdown 渲染（标题/列表正常排版，配「计划」标签）", async () => {
    installFetch();
    routes.messages = [
      msg({
        seq: 4,
        kind: "tool-call",
        content: "调用 ExitPlanMode",
        toolName: "ExitPlanMode",
        toolArgs: JSON.stringify({ plan: "# 计划标题\n\n- 第一步\n- 第二步" }),
      }),
    ];
    render(<SessionDetail session={makeSession({ status: "processing" })} onBack={() => {}} />);
    await screen.findByText("调用 ExitPlanMode");
    expandToolCall(4);
    // markdown 已渲染：# 标题 → H1，列表项 → LI（对齐既有 markdown 断言的 tagName 手法）
    expect(screen.getByText("计划标题").tagName).toBe("H1");
    expect(screen.getByText("第一步").tagName).toBe("LI");
    // 「计划」标签存在（标识这是计划正文）
    expect(screen.getByText("计划")).toBeTruthy();
  });

  it("zcode 同形覆盖：toolName 是显示 title 也能命中 plan 字段（形态识别回归锁）", async () => {
    installFetch();
    routes.messages = [
      msg({
        seq: 5,
        kind: "tool-call",
        content: "调用 制定执行计划",
        toolName: "制定执行计划",
        toolArgs: JSON.stringify({ plan: "## 方案\n\n正文段落" }),
      }),
    ];
    render(<SessionDetail session={makeSession({ status: "processing" })} onBack={() => {}} />);
    await screen.findByText("调用 制定执行计划");
    expandToolCall(5);
    expect(screen.getByText("方案").tagName).toBe("H2");
  });

  it("无 plan 字段：维持原样渲染（既有 command 断言不改）", async () => {
    installFetch();
    routes.messages = [
      msg({
        seq: 6,
        kind: "tool-call",
        content: "调用 Bash",
        toolName: "Bash",
        toolArgs: '{"command":"ls"}',
      }),
    ];
    render(<SessionDetail session={makeSession({ status: "processing" })} onBack={() => {}} />);
    await screen.findByText("调用 Bash");
    expandToolCall(6);
    expect(screen.getByTestId("tool-args-6").textContent).toBe('{"command":"ls"}');
    expect(screen.queryByText("计划")).toBeNull();
  });

  it("坏 JSON / plan 非字符串 / 空串：均原样回落，不抛错", async () => {
    installFetch();
    routes.messages = [
      msg({ seq: 7, kind: "tool-call", content: "t7", toolName: "T7", toolArgs: '{"plan":' }),
      msg({ seq: 8, kind: "tool-call", content: "t8", toolName: "T8", toolArgs: '{"plan":123}' }),
      msg({ seq: 9, kind: "tool-call", content: "t9", toolName: "T9", toolArgs: '{"plan":""}' }),
    ];
    render(<SessionDetail session={makeSession({ status: "processing" })} onBack={() => {}} />);
    await screen.findByText("调用 T7");
    expandToolCall(7);
    expect(screen.getByTestId("tool-args-7").textContent).toBe('{"plan":');
    expandToolCall(8);
    expect(screen.getByTestId("tool-args-8").textContent).toBe('{"plan":123}');
    expandToolCall(9);
    expect(screen.getByTestId("tool-args-9").textContent).toBe('{"plan":""}');
  });
});

// ==== 跳到最新（2026-09-20）：距底超阈值出现浮动按钮，点击瞬时落底 ====
describe("SessionDetail：跳到最新（2026-09-20）", () => {
  /** jsdom 无布局引擎：注入滚动几何量（既有 :841 手法），distance = 距底像素 */
  function stubGeometry(area: HTMLElement, distance: number) {
    Object.defineProperty(area, "scrollHeight", { value: 5000, configurable: true });
    Object.defineProperty(area, "clientHeight", { value: 1000, configurable: true });
    area.scrollTop = 5000 - 1000 - distance;
  }

  function renderWithMessage() {
    installFetch();
    routes.messages = [msg({ seq: 0, kind: "assistant", content: "一段回复" })];
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    return waitFor(() => screen.getByTestId("message-area"));
  }

  it("距底超阈值（>240px）出现按钮；滚回贴底消失", async () => {
    const area = await renderWithMessage();
    stubGeometry(area, 3000);
    fireEvent.scroll(area);
    expect(screen.getByTestId("jump-to-bottom")).toBeTruthy();
    stubGeometry(area, 0);
    fireEvent.scroll(area);
    expect(screen.queryByTestId("jump-to-bottom")).toBeNull();
  });

  it("点击按钮：scrollTop 瞬时落到 scrollHeight，按钮消失（正文分支）", async () => {
    const area = await renderWithMessage();
    stubGeometry(area, 3000);
    fireEvent.scroll(area);
    fireEvent.click(screen.getByTestId("jump-to-bottom"));
    expect(area.scrollTop).toBe(5000);
    expect(screen.queryByTestId("jump-to-bottom")).toBeNull();
  });

  it("距顶超阈值（>240px）出现到顶钮；滚回顶部消失", async () => {
    const area = await renderWithMessage();
    Object.defineProperty(area, "scrollHeight", { value: 5000, configurable: true });
    Object.defineProperty(area, "clientHeight", { value: 1000, configurable: true });
    area.scrollTop = 2400; // 距顶 = scrollTop > 240
    fireEvent.scroll(area);
    expect(screen.getByTestId("jump-to-top")).toBeTruthy();
    area.scrollTop = 0;
    fireEvent.scroll(area);
    expect(screen.queryByTestId("jump-to-top")).toBeNull();
  });

  it("点击到顶钮：scrollTop 瞬时落 0，按钮消失", async () => {
    const area = await renderWithMessage();
    Object.defineProperty(area, "scrollHeight", { value: 5000, configurable: true });
    Object.defineProperty(area, "clientHeight", { value: 1000, configurable: true });
    area.scrollTop = 2400;
    fireEvent.scroll(area);
    fireEvent.click(screen.getByTestId("jump-to-top"));
    expect(area.scrollTop).toBe(0);
    expect(screen.queryByTestId("jump-to-top")).toBeNull();
  });

  it("分屏（split）分支同样可用", async () => {
    installFetch();
    routes.messages = [
      msg({ seq: 0, kind: "assistant", content: "改了 /tmp/proj/src/app.rs 请看" }),
    ];
    routes.files = [fileEntry("/tmp/proj/src/app.rs")];
    routes.fileContent = "fn main() {}";
    render(<SessionDetail session={makeSession()} onBack={() => {}} />);
    fireEvent.click(await screen.findByTestId("file-link"));
    fireEvent.click(await screen.findByTestId("preview-toggle-split"));
    const area = await screen.findByTestId("message-area");
    stubGeometry(area, 3000);
    fireEvent.scroll(area);
    fireEvent.click(screen.getByTestId("jump-to-bottom"));
    expect(area.scrollTop).toBe(5000);
  });
});

// ==== 过程一键折叠（2026-09-20）：书签栏右侧开关，运行态/总结态都可用 ====
describe("SessionDetail：过程一键折叠（2026-09-20）", () => {
  function runningMessages() {
    return [
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
  }

  it("运行态（非总结模式）折叠开关出现：默认全折叠 → 一键全展 → 一键全收", async () => {
    installFetch();
    routes.messages = runningMessages();
    render(<SessionDetail session={makeSession({ status: "processing" })} onBack={() => {}} />);
    await screen.findByText("结论");
    const toggle = screen.getByTestId("process-collapse-toggle");
    // 运行态默认 thinking/tool-call 折叠 → allCollapsed=true → 动作=全部展开
    expect(toggle.getAttribute("aria-label")).toBe("展开全部过程");
    expect(screen.queryByText("内部思考内容")).toBeNull();
    fireEvent.click(toggle);
    expect(screen.getByText("内部思考内容")).toBeTruthy();
    expect(screen.getByTestId("process-collapse-toggle").getAttribute("aria-label")).toBe(
      "折叠全部过程"
    );
    // 再点全收
    fireEvent.click(screen.getByTestId("process-collapse-toggle"));
    expect(screen.queryByText("内部思考内容")).toBeNull();
  });

  it("无过程消息（纯 user/assistant）：折叠开关不渲染", async () => {
    installFetch();
    routes.messages = [
      msg({ seq: 0, kind: "user", content: "你好" }),
      msg({ seq: 1, kind: "assistant", content: "你好呀" }),
    ];
    render(<SessionDetail session={makeSession({ status: "processing" })} onBack={() => {}} />);
    await screen.findByText("你好呀");
    expect(screen.queryByTestId("process-collapse-toggle")).toBeNull();
  });
});
