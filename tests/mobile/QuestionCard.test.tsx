import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ApproveCard from "@/mobile/ApproveCard";
import QuestionCard from "@/mobile/QuestionCard";
import type { QuestionInfoView } from "@/mobile/api";

// 批次乙 T8：移动端问答卡（AskUserQuestion）。fetch 全量 stub（盖过 setup.ts 的
// msw），按 URL 分路到 session-question / session-question/answer 两族端点（注意
// 前缀包含关系：GET 路径是 POST 路径的前缀，长路径必须先判——ApproveCard 测试
// 同款教训）。组件挂载即拉问答，用例先 findBy 选项按钮就绪再交互。

/** 单选题夹具（探测档案 §3 真实 questions JSON 缩录）。
 *  `freeText` 显式给 false（= 工具未定案形态）——需要输入框的用例自行覆盖为 true。 */
function singleQuestionInfo(overrides: Partial<QuestionInfoView> = {}): QuestionInfoView {
  return {
    available: true,
    source: "mark",
    freeText: false,
    questions: [
      {
        header: "Next step",
        question: "This is a demo question — what would you like to do next?",
        multiSelect: false,
        options: [
          { label: "Tool demo", description: "Explain how AskUserQuestion works." },
          { label: "Start a task", description: "Start a coding or file task." },
          { label: "Nothing yet", description: "No further request." },
        ],
      },
    ],
    ...overrides,
  };
}

/** 多选题夹具（探测档案 §3 多选缩录） */
function multiQuestionInfo(): QuestionInfoView {
  return {
    available: true,
    source: "mark",
    freeText: false,
    questions: [
      {
        header: "Favorite fruits",
        question: "Which fruits are your favorites? (Select all that apply)",
        multiSelect: true,
        options: [
          { label: "Apple", description: "A sweet, crisp fruit." },
          { label: "Banana", description: "A soft, tropical fruit." },
          { label: "Peach", description: "A juicy summer fruit." },
        ],
      },
    ],
  };
}

interface Routes {
  question?: QuestionInfoView;
  questionStatus?: number;
  /** GET /session-question 网络失败（静默自隐分支） */
  questionNetworkFail?: boolean;
  answer?: Record<string, unknown>;
  answerStatus?: number;
  answerBody?: Record<string, unknown>;
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

/** 按 URL 分路的 fetch stub（判序：/session-question/answer 在前——GET 路径是
 *  POST 路径的前缀，反序会把 POST 吞进 GET 分支） */
function installFetch() {
  fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes("/session-question/answer")) {
      if (routes.answerStatus) {
        return new Response(JSON.stringify(routes.answerBody ?? { error: "internal" }), {
          status: routes.answerStatus,
        });
      }
      return new Response(JSON.stringify(routes.answer ?? { status: "key_sent" }), {
        status: 200,
      });
    }
    if (url.includes("/session-question")) {
      if (routes.questionNetworkFail) throw new TypeError("network down");
      if (routes.questionStatus) return new Response("no", { status: routes.questionStatus });
      return new Response(JSON.stringify(routes.question ?? singleQuestionInfo()), { status: 200 });
    }
    if (url.includes("/session-approve-options")) {
      // ApproveCard 联合断言用：问答会话上审批不可用（后端硬约束①的 UI 面）
      return new Response(
        JSON.stringify({
          available: false,
          options: [],
          verifiedWith: "test",
          currentVersion: null,
          drift: false,
        }),
        { status: 200 }
      );
    }
    throw new Error(`unexpected fetch: ${url}`);
  });
  vi.stubGlobal("fetch", fetchMock);
}

/** POST /session-question/answer 的调用 */
function answerCalls(): Array<Array<unknown>> {
  return fetchMock.mock.calls
    .filter((c: unknown[]) => /\/session-question\/answer$/.test(String(c[0])))
    .map((c) => c as unknown[]);
}

/** 放行 mock fetch 的 promise 链（若干轮微任务冲刷，足以走完 fetch→json→setState） */
async function flushAsync() {
  for (let i = 0; i < 6; i += 1) {
    await act(async () => {
      await Promise.resolve();
    });
  }
}

describe("QuestionCard：问答卡渲染与应答（批次乙 T8）", () => {
  it("单选渲染：题干（header 徽标 + question）+ 选项按钮（编号+label+description）", async () => {
    installFetch();
    routes.question = singleQuestionInfo();
    render(<QuestionCard session={{ id: "sess-1" }} />);
    expect(await screen.findByTestId("question-card")).toBeTruthy();
    expect(screen.getByTestId("question-card").getAttribute("data-mode")).toBe("single");
    expect(screen.getByTestId("question-header").textContent).toBe("Next step");
    expect(screen.getByTestId("question-text").textContent).toBe(
      "This is a demo question — what would you like to do next?"
    );
    // 编号（从 1 起）+ label + description 三要素
    expect(screen.getByTestId("question-option-0").textContent).toContain("1");
    expect(screen.getByTestId("question-option-0").textContent).toContain("Tool demo");
    expect(screen.getByTestId("question-option-0").textContent).toContain(
      "Explain how AskUserQuestion works."
    );
    expect(screen.getByTestId("question-option-1").textContent).toContain("Start a task");
    expect(screen.getByTestId("question-option-2").textContent).toContain("Nothing yet");
    // 自由文本入口：**freeText !== true 的工具**渲染「去终端作答」引导文案
    // （丁T5 §2.8 降级：序列未定案不假装能发），且**不渲染输入框**
    expect(screen.getByTestId("question-freeform-hint").textContent).toContain(
      "远程自由作答尚未实测，请在终端作答"
    );
    expect(screen.queryByTestId("question-freetext-input")).toBeNull();
    // 问答模式零 允许/拒绝 键钮
    expect(screen.queryByText("允许")).toBeNull();
    expect(screen.queryByText("拒绝")).toBeNull();
  });

  it("单选点选项：POST body {sessionId, action:'select', index}（index 为 0 起 wire 口径）→「已发送按键」终态", async () => {
    installFetch();
    routes.question = singleQuestionInfo();
    routes.answer = { status: "key_sent" };
    render(<QuestionCard session={{ id: "sess-1" }} />);
    fireEvent.click(await screen.findByTestId("question-option-1")); // UI 第 2 项
    expect(await screen.findByTestId("question-sent").then((el) => el.textContent)).toBe(
      "已发送按键"
    );
    expect(answerCalls()).toHaveLength(1);
    expect(JSON.parse(String((answerCalls()[0][1] as RequestInit).body))).toEqual({
      sessionId: "sess-1",
      action: "select",
      index: 1,
    });
    // 终态：动作按钮全部移除（零重复应答面——比 disabled 更强的防线）
    expect(screen.queryByTestId("question-option-0")).toBeNull();
    expect(screen.queryByTestId("question-option-1")).toBeNull();
    expect(screen.queryByTestId("question-cancel")).toBeNull();
    expect(screen.queryByTestId("question-freeform-hint")).toBeNull();
  });

  it("多选：点选 POST toggle + 本地勾选态切换（提交钮仅在勾选后可用）→「提交」POST submit", async () => {
    installFetch();
    routes.question = multiQuestionInfo();
    routes.answer = { status: "key_sent" };
    render(<QuestionCard session={{ id: "sess-2" }} />);
    const card = await screen.findByTestId("question-card");
    expect(card.getAttribute("data-mode")).toBe("multi");
    // 未勾选：提交钮禁用（防空提交）
    expect((screen.getByTestId("question-submit") as HTMLButtonElement).disabled).toBe(true);
    // 点选 1：POST toggle{index:0}，勾选态点亮，提交钮解锁
    fireEvent.click(screen.getByTestId("question-option-0"));
    await flushAsync();
    expect(
      screen.getByTestId("question-option-0").getAttribute("class")!.includes("bg-sky-500/20")
    ).toBe(true);
    expect((screen.getByTestId("question-submit") as HTMLButtonElement).disabled).toBe(false);
    // 点选 2 再勾一个；再点 1 取消勾选（本地态翻转）
    fireEvent.click(screen.getByTestId("question-option-2"));
    await flushAsync();
    fireEvent.click(screen.getByTestId("question-option-0"));
    await flushAsync();
    expect(
      screen.getByTestId("question-option-0").getAttribute("class")!.includes("bg-sky-500/20")
    ).toBe(false);
    expect(
      screen.getByTestId("question-option-2").getAttribute("class")!.includes("bg-sky-500/20")
    ).toBe(true);
    // toggle 不置终态：卡片仍可交互
    expect(screen.queryByTestId("question-sent")).toBeNull();
    // 提交：POST submit（后端三段式注入，前端不自行拼键）
    fireEvent.click(screen.getByTestId("question-submit"));
    expect(await screen.findByTestId("question-sent")).toBeTruthy();
    const bodies = answerCalls().map((c) => JSON.parse(String((c[1] as RequestInit).body)));
    expect(bodies).toEqual([
      { sessionId: "sess-2", action: "toggle", index: 0 },
      { sessionId: "sess-2", action: "toggle", index: 2 },
      { sessionId: "sess-2", action: "toggle", index: 0 },
      { sessionId: "sess-2", action: "submit" },
    ]);
  });

  it("取消钮：POST cancel →「已发送按键」终态", async () => {
    installFetch();
    routes.question = singleQuestionInfo();
    routes.answer = { status: "key_sent" };
    render(<QuestionCard session={{ id: "sess-3" }} />);
    fireEvent.click(await screen.findByTestId("question-cancel"));
    expect(await screen.findByTestId("question-sent")).toBeTruthy();
    expect(JSON.parse(String((answerCalls()[0][1] as RequestInit).body))).toEqual({
      sessionId: "sess-3",
      action: "cancel",
    });
  });

  it("多问题只读态：题干罗列 +「请在终端完成作答」引导，零注入按钮", async () => {
    installFetch();
    routes.question = {
      available: true,
      source: "mark",
      questions: [
        // description 恒在（后端 json! 无条件输出）——空串形态夹具
        {
          header: "A",
          question: "First?",
          multiSelect: false,
          options: [
            { label: "a1", description: "" },
            { label: "a2", description: "" },
          ],
        },
        {
          header: "B",
          question: "Second?",
          multiSelect: true,
          options: [
            { label: "b1", description: "" },
            { label: "b2", description: "" },
          ],
        },
      ],
    };
    const { container } = render(<QuestionCard session={{ id: "sess-4" }} />);
    expect(await screen.findByTestId("question-card")).toBeTruthy();
    expect(screen.getByTestId("question-card").getAttribute("data-mode")).toBe("readonly");
    expect(screen.getByTestId("question-readonly-0").textContent).toContain("First?");
    expect(screen.getByTestId("question-readonly-1").textContent).toContain("B");
    expect(screen.getByTestId("question-readonly-hint").textContent).toContain("请在终端完成作答");
    // 「结论不超证据」：多问题零注入面
    expect(screen.queryByTestId("question-option-0")).toBeNull();
    expect(screen.queryByTestId("question-submit")).toBeNull();
    expect(screen.queryByTestId("question-cancel")).toBeNull();
    expect(answerCalls()).toHaveLength(0);
    expect(container.textContent).toBeTruthy();
  });

  // 批次丙 T3：工具键序未实测（后端 answerable=false；codex 已于 2026-09-21 实机
  // 补测升格，此处用通用夹具覆盖只读档本身）→ 只读卡
  it("answerable=false（工具键序未验）：只读卡渲染题干+选项文本，零注入按钮", async () => {
    installFetch();
    routes.question = singleQuestionInfo({ answerable: false });
    const { container } = render(<QuestionCard session={{ id: "sess-codex" }} />);
    expect(await screen.findByTestId("question-card")).toBeTruthy();
    expect(screen.getByTestId("question-card").getAttribute("data-mode")).toBe("tool-readonly");
    // 题干与选项以**文本**呈现（用户能读到问题内容——JSON 裸奔问题在此消失）
    expect(screen.getByTestId("question-text").textContent).toContain("demo question");
    expect(screen.getByTestId("question-readonly-option-0").textContent).toContain("Tool demo");
    expect(screen.getByTestId("question-readonly-option-2").textContent).toContain("Nothing yet");
    expect(screen.getByTestId("question-tool-readonly-hint").textContent).toContain(
      "请在终端完成作答"
    );
    // 「未验不出键」：零注入按钮、零 POST
    expect(screen.queryByTestId("question-option-0")).toBeNull();
    expect(screen.queryByTestId("question-submit")).toBeNull();
    expect(screen.queryByTestId("question-cancel")).toBeNull();
    expect(answerCalls()).toHaveLength(0);
    expect(container.textContent).toBeTruthy();
  });

  // ===== 丁T5：进行中态 / 中止态 / 卡内自由文本 =====

  it("进行中态：请求在途期间显示「进行中」而不是「已发送按键」，完成后转终态", async () => {
    installFetch();
    routes.question = multiQuestionInfo();
    // **受控 promise**：只把 **submit** 的响应挂在手里（toggle 立即成功——
    // 否则提交钮一直是禁用态，看不到在途帧），这样才看得到「在途」那一帧
    let releaseSubmit!: (v: Response) => void;
    const pendingSubmit = new Promise<Response>((resolve) => {
      releaseSubmit = resolve;
    });
    fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.includes("/session-question/answer")) {
        const body = JSON.parse(String(init?.body ?? "{}")) as { action?: string };
        if (body.action === "submit") return pendingSubmit;
        return new Response(JSON.stringify({ status: "key_sent" }), { status: 200 });
      }
      return new Response(JSON.stringify(multiQuestionInfo()), { status: 200 });
    });
    vi.stubGlobal("fetch", fetchMock);
    render(<QuestionCard session={{ id: "sess-t5a" }} />);
    await screen.findByTestId("question-card");
    fireEvent.click(screen.getByTestId("question-option-0"));
    await flushAsync();
    fireEvent.click(screen.getByTestId("question-submit"));
    // **在途帧**：进行中态在场（段名 = submit-row），终态文案不在
    const progress = await screen.findByTestId("question-progress");
    expect(progress.textContent).toContain("正在定位提交入口");
    expect(progress.getAttribute("data-stage")).toBe("submit-row");
    expect(screen.queryByTestId("question-sent")).toBeNull();
    // 放行响应 → 转终态
    releaseSubmit(
      new Response(
        JSON.stringify({ status: "key_sent", done: true, stage: "receipt", verified: true }),
        { status: 200 }
      )
    );
    expect(await screen.findByTestId("question-sent")).toBeTruthy();
    expect(screen.queryByTestId("question-progress")).toBeNull();
    expect(screen.queryByTestId("question-verified-unseen")).toBeNull();
  });

  it("阶段机中止：显示中止段 + 原因 + 引到终端（可重试），不是笼统的「已发送按键」", async () => {
    installFetch();
    routes.question = multiQuestionInfo();
    routes.answer = {
      status: "failed",
      aborted: true,
      stage: "review",
      error:
        "已发回车但屏上未出现 Review 确认屏（未见「review your answers」/「ready to submit」）——已中止，未发确认键；请人工核对终端",
    };
    render(<QuestionCard session={{ id: "sess-t5b" }} />);
    await screen.findByTestId("question-card");
    fireEvent.click(screen.getByTestId("question-option-0"));
    await flushAsync();
    fireEvent.click(screen.getByTestId("question-submit"));
    // 中止段名 + 后端整句原因 + 引到终端的提示
    expect((await screen.findByTestId("question-aborted-stage")).textContent).toContain(
      "等待确认屏"
    );
    expect(screen.getByTestId("question-error").textContent).toContain("Review 确认屏");
    expect(screen.getByTestId("question-aborted-hint").textContent).toContain(
      "请到终端查看当前对话框状态后重试"
    );
    // **可重试**：不置终态（提交钮仍在，按钮未锁死）
    expect(screen.queryByTestId("question-sent")).toBeNull();
    expect(screen.getByTestId("question-submit")).toBeTruthy();
  });

  it("阶段机走完但未见终态回执：如实提示「请到终端确认结果」（不谎报完成）", async () => {
    installFetch();
    routes.question = multiQuestionInfo();
    routes.answer = { status: "key_sent", done: true, stage: "receipt", verified: false };
    render(<QuestionCard session={{ id: "sess-t5c" }} />);
    await screen.findByTestId("question-card");
    fireEvent.click(screen.getByTestId("question-option-0"));
    await flushAsync();
    fireEvent.click(screen.getByTestId("question-submit"));
    expect(await screen.findByTestId("question-sent")).toBeTruthy();
    expect(screen.getByTestId("question-verified-unseen").textContent).toContain(
      "未在屏上见到完成回执"
    );
  });

  it("卡内自由文本（freeText=true 的工具）：输入框 + 「作为回答发送」→ POST freeText{text}", async () => {
    installFetch();
    routes.question = singleQuestionInfo({ freeText: true });
    routes.answer = { status: "key_sent", done: true, stage: "free-text", verified: true };
    render(<QuestionCard session={{ id: "sess-t5d" }} />);
    await screen.findByTestId("question-card");
    // 输入框与发送钮在场；空文本时发送钮禁用（防空提交）
    const input = (await screen.findByTestId("question-freetext-input")) as HTMLInputElement;
    expect((screen.getByTestId("question-freetext-send") as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(input, { target: { value: "green tea please" } });
    expect((screen.getByTestId("question-freetext-send") as HTMLButtonElement).disabled).toBe(
      false
    );
    fireEvent.click(screen.getByTestId("question-freetext-send"));
    expect(await screen.findByTestId("question-sent")).toBeTruthy();
    // POST body：action=freeText + text 原文（归一在后端；前端不加工）
    const bodies = answerCalls().map((c) => JSON.parse(String((c[1] as RequestInit).body)));
    expect(bodies).toEqual([
      { sessionId: "sess-t5d", action: "freeText", text: "green tea please" },
    ]);
    // 成功后输入框清空（已投递；留着会让用户以为没发出去）——但终态下输入框整块不渲染
    expect(screen.queryByTestId("question-freetext-input")).toBeNull();
  });

  // 复评 F6-3：多选卡**不提供**自由文本入口——多选屏的自由作答行带勾选框
  // （`4. [ ] Type something`，实机截图 C-s8-cursor-submit-*.png 第 4 行），而定位判据
  // 是「剥编号后以 `Type something` 开头」→ 不匹配 → 定位恒失败 → 后端恒拒 409。
  // 前端同步不渲染输入框（不给用户一个必然失败的按钮），改渲染终端引导。
  // **即使后端 `freeText:true`（工具支持）也不渲染**——这是**题目形态**维度的限制。
  it("多选卡不提供自由文本入口（复评 F6-3）：freeText=true 也不渲染输入框，改渲染终端引导", async () => {
    installFetch();
    routes.question = multiQuestionInfo();
    routes.question.freeText = true; // 工具支持，但题目是多选 → 仍不给
    render(<QuestionCard session={{ id: "sess-t5e" }} />);
    await screen.findByTestId("question-card");
    // 勾选路与提交钮照常（多选的主路径）
    expect(await screen.findByTestId("question-option-0")).toBeTruthy();
    expect(screen.getByTestId("question-submit")).toBeTruthy();
    // 自由文本入口**不在场**，走降级引导文案
    expect(screen.queryByTestId("question-freetext-input")).toBeNull();
    expect(screen.queryByTestId("question-freetext-send")).toBeNull();
    expect(screen.getByTestId("question-freeform-hint").textContent).toContain(
      "多选题请到终端作答"
    );
    // 零注入：不得有任何应答请求
    expect(answerCalls()).toHaveLength(0);
  });

  it("freeText 缺省（旧后端 / 未定案工具）：不渲染输入框，渲染终端引导文案", async () => {
    installFetch();
    // 夹具不带 freeText 字段（旧后端形态）
    const info = singleQuestionInfo();
    delete (info as { freeText?: boolean }).freeText;
    routes.question = info;
    render(<QuestionCard session={{ id: "sess-t5f" }} />);
    await screen.findByTestId("question-card");
    expect(screen.queryByTestId("question-freetext-input")).toBeNull();
    expect(screen.queryByTestId("question-freetext-send")).toBeNull();
    expect(screen.getByTestId("question-freeform-hint").textContent).toContain("请在终端作答");
  });

  it("ApiError tool_readonly：分診为「该工具的远程作答尚未实测」并引到终端", async () => {
    installFetch();
    routes.question = singleQuestionInfo({ freeText: true });
    routes.answerStatus = 409;
    routes.answerBody = { error: "tool_readonly" };
    render(<QuestionCard session={{ id: "sess-t5g" }} />);
    await screen.findByTestId("question-card");
    fireEvent.change(await screen.findByTestId("question-freetext-input"), {
      target: { value: "hi" },
    });
    fireEvent.click(screen.getByTestId("question-freetext-send"));
    expect((await screen.findByTestId("question-error")).textContent).toContain(
      "该工具的远程作答尚未实测，请在终端完成作答"
    );
  });

  it("answerable 缺省（旧后端）：仍走可作答路径（前向兼容）", async () => {
    installFetch();
    // 夹具不带 answerable 字段
    render(<QuestionCard session={{ id: "sess-legacy" }} />);
    expect(await screen.findByTestId("question-option-0")).toBeTruthy();
    expect(screen.queryByTestId("question-tool-readonly-hint")).toBeNull();
  });

  it("available=false / 拉取失败：组件自隐（container empty）", async () => {
    installFetch();
    routes.question = singleQuestionInfo({ available: false, questions: [] });
    const { container } = render(<QuestionCard session={{ id: "sess-5" }} />);
    await flushAsync();
    expect(container.firstElementChild).toBeNull();
    cleanup();
    routes.questionNetworkFail = true;
    const { container: c2 } = render(<QuestionCard session={{ id: "sess-5" }} />);
    await flushAsync();
    expect(c2.firstElementChild).toBeNull();
  });

  it("failed{error}：错误文案展示且可重试（重试转「已发送按键」）", async () => {
    installFetch();
    routes.question = singleQuestionInfo();
    routes.answer = { status: "failed", error: "投递进行中，请稍后重试" };
    render(<QuestionCard session={{ id: "sess-6" }} />);
    fireEvent.click(await screen.findByTestId("question-option-0"));
    expect(await screen.findByTestId("question-error").then((el) => el.textContent)).toContain(
      "投递进行中，请稍后重试"
    );
    // 可重试：按钮保持可点，修正路由后重按即重试成功
    expect((screen.getByTestId("question-option-0") as HTMLButtonElement).disabled).toBe(false);
    routes.answer = { status: "key_sent" };
    fireEvent.click(screen.getByTestId("question-option-0"));
    expect(await screen.findByTestId("question-sent")).toBeTruthy();
    expect(answerCalls()).toHaveLength(2);
  });

  it("ApiError 分診：409 no_question / 409 multi_questions / 400 bad_index 中文文案", async () => {
    installFetch();
    routes.question = singleQuestionInfo();
    const cases: Array<[number, Record<string, unknown>, string]> = [
      [409, { error: "no_question" }, "当前没有待回答的问题"],
      [409, { error: "multi_questions" }, "多个问题请回到终端完成作答"],
      [400, { error: "bad_index" }, "选项序号无效，请刷新后重试"],
    ];
    for (const [status, body, copy] of cases) {
      routes.answerStatus = status;
      routes.answerBody = body;
      const { unmount } = render(<QuestionCard session={{ id: "sess-7" }} />);
      fireEvent.click(await screen.findByTestId("question-option-0"));
      expect(await screen.findByTestId("question-error").then((el) => el.textContent)).toContain(
        copy
      );
      unmount();
      cleanup();
    }
  });
});

// ==== 联合断言：问答模式不渲染 允许/拒绝 + key 前缀互异 ====
describe("QuestionCard 与 ApproveCard 联合（硬约束① UI 面）", () => {
  it("问答会话上 ApproveCard 自隐（approve 选项不可用即 null）、QuestionCard 在场且无允许/拒绝", async () => {
    installFetch();
    // 同层挂载（SessionDetail 同构：approve-*/question-*/composer- 前缀互异）
    const { container } = render(
      <div>
        <ApproveCard key="approve-sess-8" session={{ id: "sess-8" }} />
        <QuestionCard key="question-sess-8" session={{ id: "sess-8" }} />
      </div>
    );
    // 问答卡在场（单选渲染就绪）
    expect(await screen.findByTestId("question-card")).toBeTruthy();
    // ApproveCard 拉取 available=false → 卡自隐（既有逻辑，未改坏）
    await flushAsync();
    expect(screen.queryByTestId("approve-card")).toBeNull();
    // 问答卡内零 允许/拒绝 键钮（问答模式映射键位无意义）
    expect(screen.queryByText("允许")).toBeNull();
    expect(screen.queryByText("拒绝")).toBeNull();
    expect(container.textContent).toContain("等待回答");
  });

  it("key 前缀防线：approve-*/question-* 前缀字符串互异（SessionDetail 挂载契约的静态锁）", async () => {
    // 联合挂载同会话双卡无 duplicate key 警告（React key 冲突会 console.error——
    // vitest setup 将 error 抛出即本测红）；前缀本身由 SessionDetail 源码承载
    installFetch();
    routes.question = singleQuestionInfo({ available: false, questions: [] });
    render(
      <div>
        <ApproveCard key="approve-sess-9" session={{ id: "sess-9" }} />
        <QuestionCard key="question-sess-9" session={{ id: "sess-9" }} />
      </div>
    );
    await flushAsync();
    // 两卡各自独立 fetch（互不串载）：approve-options 与 session-question 各一次
    const urls = fetchMock.mock.calls.map((c: unknown[]) => String(c[0]));
    expect(urls.some((u) => u.includes("/session-approve-options"))).toBe(true);
    expect(urls.some((u) => u.includes("/session-question"))).toBe(true);
  });
});

// ==== 批次戊 E4：多题交互卡（multiQuestion=true → 逐题作答 + 末题提交）====
describe("QuestionCard：E4 多题交互（multiQuestion 旗标）", () => {
  /** 两题夹具（E4：kimi/codex/opencode 的多题交互面） */
  function twoQuestionInteractive(): QuestionInfoView {
    return {
      available: true,
      source: "mark",
      multiQuestion: true,
      questions: [
        {
          header: "A",
          question: "First?",
          multiSelect: false,
          options: [
            { label: "a1", description: "" },
            { label: "a2", description: "" },
          ],
        },
        {
          header: "B",
          question: "Second?",
          multiSelect: true,
          options: [
            { label: "b1", description: "" },
            { label: "b2", description: "" },
          ],
        },
      ],
    };
  }

  it("逐题作答：select 携 questionIndex 推进，末题出现提交钮 → submit", async () => {
    installFetch();
    routes.question = twoQuestionInteractive();
    routes.answer = { status: "key_sent" };
    render(<QuestionCard session={{ id: "sess-e4-mq" }} />);
    await screen.findByTestId("question-multi-current");
    expect(screen.getByTestId("question-multi-current").textContent).toContain("First?");
    // 首题无提交钮（未到末题）
    expect(screen.queryByTestId("question-multi-submit")).toBeNull();
    // 答第 1 题 → 本地推进到第 2 题（questionIndex=0 上行）
    fireEvent.click(screen.getByTestId("question-multi-option-0"));
    await flushAsync();
    expect(screen.getByTestId("question-multi-current").textContent).toContain("Second?");
    // 末题：提交钮出现；答末题 → 终态（sent）+ 提交钮消失
    expect(screen.getByTestId("question-multi-submit")).toBeTruthy();
    fireEvent.click(screen.getByTestId("question-multi-option-1"));
    await flushAsync();
    expect(await screen.findByTestId("question-sent")).toBeTruthy();
    const bodies = answerCalls().map((c) => JSON.parse(String((c[1] as RequestInit).body)));
    expect(bodies).toEqual([
      { sessionId: "sess-e4-mq", action: "select", index: 0, questionIndex: 0 },
      { sessionId: "sess-e4-mq", action: "select", index: 1, questionIndex: 1 },
    ]);
  });

  it("multiQuestion 缺省（旧后端）→ 维持只读卡", async () => {
    installFetch();
    routes.question = {
      available: true,
      source: "mark",
      questions: [
        {
          header: "A",
          question: "First?",
          multiSelect: false,
          options: [
            { label: "a1", description: "" },
            { label: "a2", description: "" },
          ],
        },
        {
          header: "B",
          question: "Second?",
          multiSelect: false,
          options: [
            { label: "b1", description: "" },
            { label: "b2", description: "" },
          ],
        },
      ],
    };
    render(<QuestionCard session={{ id: "sess-e4-ro" }} />);
    expect(await screen.findByTestId("question-readonly-hint")).toBeTruthy();
    expect(screen.queryByTestId("question-multi-option-0")).toBeNull();
  });
});
