// 移动端问答卡（批次乙 T8，AskUserQuestion 问答卡 · claude 先行）：挂在
// SessionDetail 正文视图 / 分屏对话列（**非结束态**；丁T1 放宽——ApproveCard 仍是
// waiting 门，key 前缀 question-* 与 approve-*/composer-* 互异防 duplicate-key）。
// - 可用性：挂载拉取一次 /session-question；**状态跃迁重拉**（丁T1 复评 F-1：
//   effect deps 含 `session.status`——详情页停留期间 Board 的既有数据通道
//   （SSE 跃迁/快照 + 降级轮询）把活会话 status 对齐进 selected，status 一变即
//   重拉一次：终端答完题（waiting → processing/idle）卡随之消失，新问题出现
//   （→ waiting）卡随之浮现；无需重进页面）；拉取失败 / 网络异常 → 静默自隐；
//   available=false（双通道未命中 / 审批标记隔离）→ 自隐（fetchApproveOptions 惯例）；
// - **问答模式不出 允许/拒绝**（ApproveCard 的映射键位对问答无意义——后端硬约束①
//   同时保证问答会话上 approve 端点不可用，红卡自隐；本卡自身也零允许/拒绝字样）；
// - 渲染：题干（header 徽标 + question 文本）+ 选项按钮（编号+label+description，
//   编号从 1 起）；「取消」钮 = Esc（探测 K3：取消/拒绝整个问题）；
// - 单选（multiSelect=false）：点选项 → POST select{index}（后端注入对应数字单键——
//   探测 K1/K2 定案：数字直接勾选并提交，无回车、严禁后补 Esc）；
// - 多选（multiSelect=true）：点选 = POST toggle{index}（后端注入数字切换勾选，探测
//   K8）+ 本地勾选态（仅在成功回执后切换，防端点拒绝时本地状态漂移）+「提交」钮 →
//   POST submit（后端三段式：down×(n+1) → enter → '1'，探测 K10——Enter 当提交是
//   反直觉反例 K9，前端绝不自行拼 Enter）；
// - **丁T5 起三段式改为后端阶段机闭环**（§2.3 裁4）：submit 的响应带 `done`/`stage`/
//   `verified`——走完整条（提交行 → 走位 → 回车 → Review 屏 → 抄屏上编号确认 → 终态）
//   才显示完成；中途任一段屏读不符 → `failed{aborted:true, stage}`，卡片显示**中止在
//   哪一段 + 原因 + 引到终端**（不是笼统的「已发送按键」）；
// - **进行中态**（§2.3「卡片进行中态替代『已发送按键』」）：请求在途期间卡片显示
//   「进行中（走到哪一段）」——后端的段推进是同步的（一次请求内走完），故前端只需
//   一个总进行中态 + 段名文案（`QUESTION_STAGE_LABELS`）；
// - 自由文本（**丁T5 §2.4 入口 1；复评 F6-3 收紧为「仅单选单题卡 + 仅 claude」**）：
//   卡内嵌输入框 + 「作为回答发送」→ POST freeText{text}。后端序列 = 定位
//   `Type something` 行（数字，仅移动焦点）→ 文本（**字符通道**）→ 回车；文本经归一
//   且**不带** `[mobile]` 签名。
//   **两个不渲染输入框的情形**（都渲染「请在终端作答」引导，**不假装能发**）：
//   ① 工具未定案（codex/kimi/opencode/未知；`info.freeText !== true`，§2.8）；
//   ② **多选题**（复评 F6-3）：多选屏的自由作答行带勾选框
//      （`4. [ ] Type something`，实机截图 `C-s8-cursor-submit-*.png`），定位判据
//      （剥编号后以 `Type something` 开头）不匹配 → 后端恒拒 409；前端同步不给按钮。
//      证据与放宽前提见 `inject::question::free_text_shape_supported` 的文档；
// - 多问题（questions.length>1）：**只读卡**——题干罗列 +「请在终端完成作答」引导，
//   零注入按钮（翻页键序未测，「结论不超证据」；后端 answer 端点同样拒绝）；
// - 应答分診：key_sent → 终态（按钮禁用）——阶段机动作另显 `verified` 的三态；failed
//   {error} → 错误文案可重试（`aborted:true` 时额外显示段名 + 引到终端）；ApiError
//   （409/400 带 data.error）→ 分診中文文案：no_question→「当前没有待回答的问题」、
//   multi_questions→「多个问题请回到终端完成作答」、tool_readonly→「该工具的远程作答
//   尚未实测，请在终端完成作答」、bad_index→「选项序号无效，请刷新后重试」、
//   其余显示 message。
// **已知限制（丁T1 复评 F-1，如实申报）**：重拉只由**状态跃迁**驱动，不做卡内轮询
// （轮询超 T1 范围，丁T2 另有安排）。因此同一 waiting 窗口内的非跃迁变化——例如
// 模型连续提两组问题、或用户改答但状态未变——不会自动重拉，需等下一次状态跃迁
// （或重进详情页）。`session.id` 变化会重拉（跨会话串卡防线，`key` 也随 id 强制重挂）。
// **已知限制之二（丁T1 复审 F2-3，opencode 的载荷就绪空窗）**：opencode 的 question
// part 是「part 行先落盘、题目数据后到」——`pending` 事件里 `state.input` 恒为 `{}`
// （questions 尚未写入），要到 `running` 拍才有。实测空窗 **5ms–4s**（数据源见
// `monitor::opencode_parser::pending_question_part` 文档）。影响面：
// - **红灯不受影响**：状态链用**工具名**分支判待决（`input` 为空也成立）；
// - **卡片在这一拍确实无法渲染**：端点拿不到 questions，`available=false` → 卡自隐
//   （没有任何题目文本可显示，不是逻辑错，是数据未就绪）；
// - 卡片会在下一次**状态跃迁**驱动的重拉时出现（同属上一条「非跃迁变化不自动重拉」
//   的限制族）。**不为它加占位卡或轮询**（超 T1 范围；重拉触发机制归丁T2 收口面）。
import { useCallback, useEffect, useRef, useState } from "react";
import InteractiveCard, { toneTokens } from "./InteractiveCard";
import {
  ApiError,
  fetchSessionQuestion,
  questionAnswerErrorCopy,
  sessionQuestionAnswer,
  type QuestionAnswerAction,
  type QuestionAnswerStage,
  type QuestionInfoView,
} from "./api";

/** 阶段名 → 用户可读文案（**进行中态与中止回执共用**，两处不会漂移）。
 *  取值与后端 `remote::api::QUESTION_STAGE_*` 逐字对应（见 api.ts 的
 *  `QuestionAnswerStage` 注释）。 */
const QUESTION_STAGE_LABELS: Record<QuestionAnswerStage, string> = {
  "submit-row": "定位提交入口",
  review: "等待确认屏",
  confirm: "确认提交",
  receipt: "核对完成回执",
  "free-row": "定位自由作答行",
  "free-text": "提交回答文本",
};

/** 阶段名 → 进行中文案（比 `QUESTION_STAGE_LABELS` 更像「正在做什么」——
 *  同一段在「进行中」与「中止」两个语境里的措辞不同，两套文案都在本文件内。 */
const QUESTION_STAGE_PROGRESS: Record<QuestionAnswerStage, string> = {
  "submit-row": "正在定位提交入口（屏读确认）…",
  review: "已提交勾选，正在等待确认屏…",
  confirm: "确认屏已出现，正在确认提交…",
  receipt: "正在核对完成回执…",
  "free-row": "正在定位自由作答输入行…",
  "free-text": "正在提交回答文本…",
};

/** 自由作答文本长度上限（与 composer 的 `MAX_SEND_CHARS` 对齐；后端同口径 400） */
const MAX_FREE_TEXT_CHARS = 10000;

/** 问答载荷的**内容指纹**（丁T1 复评 F2-1，纯函数）：题干 + 每题的选项标签与顺序
 *  + 多选标记 + 选项描述的组合摘要。同一问题重复拉取恒等；模型换题或改选项即变。
 *  为何不用 `JSON.stringify(info)` 直接比：载荷里含 `source`（"mark"/"scan"）——
 *  同一问题从标记通道落到扫描通道会翻字符串但问题并未变化，那不该重置终态。 */
function questionFingerprint(info: QuestionInfoView): string {
  return info.questions
    .map((q) =>
      [
        q.header,
        q.question,
        q.multiSelect ? "m" : "s",
        q.options.map((o) => `${o.label}\u0001${o.description}`).join("\u0002"),
      ].join("\u0003")
    )
    .join("\u0004");
}

interface QuestionCardProps {
  /** 会话：只消费 id（请求键）与 status（重拉触发键，丁T1 复评 F-1）。
   *  结构化类型——完整 Session 可直接传入，测试可只给这两字段 */
  session: { id: string; status?: string };
}

export default function QuestionCard({ session }: QuestionCardProps) {
  // 可用性：ready=false（加载中 / 拉取失败）→ 不渲染
  const [info, setInfo] = useState<QuestionInfoView | null>(null);
  const [ready, setReady] = useState(false);
  // 应答进行中（防连点）
  const [busy, setBusy] = useState(false);
  // **进行中态**（丁T5 §2.3）：请求在途期间显示「进行中（走到哪一段）」——
  // 阶段机动作（submit/freeText）是**一条同步请求内走完整条闭环**的，故前端拿不到
  // 中间段；这里显示的是「已发起的动作」，段名文案按动作类型取首段（`stage` 字段
  // 只有中止时才有真值）。请求返回即被终态或中止态取代。
  const [inProgress, setInProgress] = useState<QuestionAnswerStage | null>(null);
  // 多选本地勾选态（仅在 toggle 成功回执后切换——端点拒绝时本地状态不漂移）
  const [checked, setChecked] = useState<Set<number>>(() => new Set());
  // 终态：按键序列已投递（key_sent）——按钮禁用 +「已发送按键」。**toggle 不算终态**
  // （多选点选后仍需「提交」，置终态会锁死提交钮）
  const [sent, setSent] = useState(false);
  // 阶段机走完全链后的**终态回执核验**（丁T5）：true=屏读到终态锚（确认完成）；
  // false=读到屏但未见锚（不谎报，提示人工核对）；null/undefined=读屏不可用。
  // 非阶段机动作（select/toggle/cancel）恒 null（无此语义）。
  const [verified, setVerified] = useState<boolean | null>(null);
  // 失败文案（failed{error} 回执 / ApiError 分診）——非 null 展示，按钮保持可点
  const [error, setError] = useState<string | null>(null);
  // 中止的段名（丁T5：`failed{aborted:true, stage}`）——与 `error` 并存：
  // error 是后端的整句中文说明，stage 供渲染「卡在哪一段」的进度语义
  const [abortedStage, setAbortedStage] = useState<QuestionAnswerStage | null>(null);
  // 自由作答输入框内容（**仅单题卡 + info.freeText === true 时渲染**）
  const [freeText, setFreeText] = useState("");
  // E4-E6 多题交互：当前作答到第几题（0 起；answer 成功且非末题时 +1）
  const [mqIndex, setMqIndex] = useState(0);

  // 拉取（挂载一次 + 状态跃迁重拉，丁T1 复评 F-1）：deps 含 `session.status`——
  // 详情页停留期间 Board 数据通道把活会话 status 对齐进 selected（App.tsx
  // handleSessionsChanged），status 一变即重拉：答完题卡消失、新问题卡浮现。
  // 「非状态跃迁的变更不自动重拉」是已知限制（见文件头注释）。
  const status = session.status;
  // 上一轮载荷的**内容指纹**（丁T1 复评 F2-1）：用于判定「这一轮拿到的是不是新问题」
  const lastFingerprint = useRef<string | null>(null);
  useEffect(() => {
    let alive = true;
    setReady(false);
    // 重拉时清掉上一轮的错误文案（陈旧「没有待回答的问题」会误导新一轮）
    setError(null);
    fetchSessionQuestion(session.id)
      .then((v) => {
        if (!alive) return;
        // **问题内容变化才重置终态/勾选态**（F2-1，2026-09-21 复评）：
        // 同一会话内可连续多次提问（实测 rollout-2026-09-21T13-44-08：单会话连续
        // 8 次 request_user_input，两两之间无 task_complete），而 key 是
        // `question-${session.id}` → **不重挂** → `sent=true` 会残留到下一题，
        // 用户看到一张写着「已发送按键」且无按钮的**伪终态**卡。
        // 为什么不用「无条件清」：投递成功（key_sent）到状态回落之间有短暂窗口，
        // 期间 `sent` 必须保留以**防连投**（同一次问答内重复按键会二次投递终端）。
        // 故判据取「内容变了才是新问题」——指纹 = 题目结构摘要（题干 + 选项标签
        // + 多选标记），对同一问题的重复拉取稳定不变。
        //
        // 边界：**不可用载荷不参与判据**（`available=false` → 指纹视为空串）——
        // 「拉不到题」与「换了题」是两回事：opencode 的 pending 拍 input 未就绪
        // （F2-3）就会短暂 available=false，若让它算「内容变化」，会把刚投递的
        // sent 清掉 → 按钮复活 → 防连投语义被削弱。
        const fp = v.available && v.questions.length > 0 ? questionFingerprint(v) : "";
        // 只在「两次都是可用载荷且内容不同」时重置（空串一律不触发重置）
        if (
          lastFingerprint.current !== null &&
          lastFingerprint.current !== "" &&
          fp !== "" &&
          lastFingerprint.current !== fp
        ) {
          setSent(false);
          setChecked(new Set());
          // 新问题的输入框清空（旧答案不该跟着新题走）
          setFreeText("");
          setVerified(null);
          setAbortedStage(null);
          setMqIndex(0);
        }
        if (fp !== "") lastFingerprint.current = fp;
        setInfo(v);
        setReady(true);
      })
      .catch(() => {
        if (alive) setReady(false);
      });
    return () => {
      alive = false;
    };
  }, [session.id, status]);

  const handleAnswer = useCallback(
    async (
      action: QuestionAnswerAction,
      index?: number,
      text?: string,
      /** E4-E6 多题交互：select/toggle 作用在第几题（0 起） */
      questionIndex?: number
    ) => {
      if (busy || sent) return;
      setBusy(true);
      setError(null);
      setAbortedStage(null);
      // **进行中态**（丁T5）：按动作显示对应的首段文案（提交链/自由作答链）
      setInProgress(action === "freeText" ? "free-row" : "submit-row");
      try {
        const res = await sessionQuestionAnswer(session.id, action, index, text, questionIndex);
        if (res.status === "key_sent") {
          if (action === "toggle" && typeof index === "number") {
            // 多选勾选切换：成功回执后翻本地位（下轮渲染高亮）；不置终态
            setChecked((prev) => {
              const next = new Set(prev);
              if (next.has(index)) {
                next.delete(index);
              } else {
                next.add(index);
              }
              return next;
            });
          } else if (
            action === "select" &&
            typeof questionIndex === "number" &&
            info !== null &&
            questionIndex < info.questions.length - 1
          ) {
            // E4-E6 多题逐题推进：本题数字已发（kimi DigitAdvance 自动切下一题），
            // 本地切到下一题继续作答——**不置终态**（末题 select / submit 才终态）
            setMqIndex(questionIndex + 1);
          } else {
            // select / submit / cancel / freeText：终态
            // （select=数字已提交 / submit=阶段机走完 / cancel=已取消 / freeText=文本已提交）
            setSent(true);
            // 阶段机动作带回 verified（三态）；单键动作无该字段 → 保持 null
            setVerified(typeof res.verified === "boolean" ? res.verified : null);
            // 自由作答成功后清空输入框（已投递；留着会让用户以为没发出去）
            if (action === "freeText") setFreeText("");
          }
        } else {
          // failed：区分「阶段机中止」（aborted+stage）与普通投递失败（可重试）
          setError(res.error);
          if (res.aborted === true && res.stage) setAbortedStage(res.stage);
        }
      } catch (e) {
        if (e instanceof ApiError) {
          // 错误码 → 中文文案走**单点映射**（`questionAnswerErrorCopy`）：
          // composer 的问答转向路径用同一个函数，两条入口不会漂移（丁T6 复评抽出）
          setError(questionAnswerErrorCopy(e));
        } else {
          setError(String(e));
        }
      } finally {
        setBusy(false);
        setInProgress(null);
      }
    },
    [busy, sent, session.id, info]
  );

  // 加载中 / 拉取失败 / info 未落地 / 不可用：不渲染（卡自隐）
  if (!ready || info === null || !info.available) return null;
  const questions = info.questions;
  if (questions.length === 0) return null;

  // T3：该工具的问答键序未实测（answerable 明确 false，如 codex——实机 0 样本，
  // 键位仅源码级）→ 只读卡 + 引导终端作答（「未验不出键」；渲染选项供阅读，
  // 但不给可点按钮）。`answerable` 缺省按 true（前向兼容旧后端）
  if (info.answerable === false) {
    const q0 = questions[0];
    return (
      <InteractiveCard
        tone="question"
        testId="question-card"
        mode="tool-readonly"
        pulsing={false}
        title="等待回答"
      >
        {q0.header && (
          <div
            data-testid="question-header"
            className="mt-1.5 inline-block rounded bg-sky-500/10 px-1.5 py-0.5 text-[10px] font-medium text-sky-700 dark:bg-sky-400/10 dark:text-sky-400"
          >
            {q0.header}
          </div>
        )}
        <p data-testid="question-text" className="mt-1 text-sm text-slate-800 dark:text-slate-200">
          {q0.question}
        </p>
        <ol className="mt-1.5 space-y-0.5">
          {q0.options.map((o, i) => (
            <li
              key={`question-ro-opt-${i}`}
              data-testid={`question-readonly-option-${i}`}
              className="text-xs text-slate-700 dark:text-slate-300"
            >
              <span className="mr-1 font-mono text-slate-500 dark:text-slate-400">{i + 1}.</span>
              {o.label}
              {o.description && (
                <span className="ml-1 text-slate-500 dark:text-slate-400">— {o.description}</span>
              )}
            </li>
          ))}
        </ol>
        <p
          data-testid="question-tool-readonly-hint"
          className="mt-1.5 text-xs text-sky-700 dark:text-sky-400"
        >
          该工具的远程作答尚未实测，请在终端完成作答
        </p>
      </InteractiveCard>
    );
  }

  // 多问题（批次戊 E4-E6）：键序已实机定案的三家（multiQuestion 旗标）→ **逐题交互
  // 卡**——数字直选逐题推进（kimi DigitAdvance 自动切下一题 / codex 数字即答 /
  // opencode tab 切页），答完全部题目出现「提交答案」钮（后端阶段机走 Review 汇总
  // 屏闭环）。未定案工具维持只读（零注入按钮）。
  if (questions.length > 1) {
    if (info.multiQuestion !== true) {
      return (
        <InteractiveCard
          tone="question"
          testId="question-card"
          mode="readonly"
          pulsing={false}
          title={`有 ${questions.length} 个问题等待回答`}
        >
          <ol className="mt-1.5 space-y-1">
            {questions.map((q, i) => (
              <li
                key={`question-readonly-${i}`}
                data-testid={`question-readonly-${i}`}
                className="text-xs text-slate-700 dark:text-slate-300"
              >
                {q.header && (
                  <span className="mr-1 rounded bg-sky-500/10 px-1 py-0.5 text-[10px] font-medium text-sky-700 dark:bg-sky-400/10 dark:text-sky-400">
                    {q.header}
                  </span>
                )}
                {q.question}
              </li>
            ))}
          </ol>
          <p
            data-testid="question-readonly-hint"
            className="mt-1.5 text-xs text-sky-700 dark:text-sky-400"
          >
            请在终端完成作答
          </p>
        </InteractiveCard>
      );
    }
    // 逐题交互：mqIndex = 当前作答的题（0 起）；全部答完 → 提交钮
    const q = questions[Math.min(mqIndex, questions.length - 1)];
    const isLast = mqIndex >= questions.length - 1;
    return (
      <InteractiveCard
        tone="question"
        testId="question-card"
        mode="multi"
        pulsing={false}
        title={`有 ${questions.length} 个问题等待回答（第 ${mqIndex + 1} 题）`}
        footer={
          <>
            {error !== null && (
              <p
                data-testid="question-error"
                className="mt-1 text-xs text-rose-600 dark:text-rose-400"
              >
                {error}
              </p>
            )}
            {abortedStage !== null && (
              <p
                data-testid="question-aborted"
                className="mt-1 text-xs text-amber-700 dark:text-amber-400"
              >
                卡在阶段：{QUESTION_STAGE_LABELS[abortedStage] ?? abortedStage}
              </p>
            )}
            {sent && (
              <p
                data-testid="question-sent"
                className="mt-1.5 text-xs font-medium text-emerald-600 dark:text-emerald-400"
              >
                已发送按键
              </p>
            )}
          </>
        }
      >
        <p
          data-testid="question-multi-current"
          className="mt-1 text-xs text-slate-700 dark:text-slate-300"
        >
          {q.header && (
            <span className="mr-1 rounded bg-sky-500/10 px-1 py-0.5 text-[10px] font-medium text-sky-700 dark:bg-sky-400/10 dark:text-sky-400">
              {q.header}
            </span>
          )}
          {q.question}
        </p>
        <div className="mt-1.5 space-y-1">
          {q.options.map((o, i) => (
            <button
              key={`mq-${mqIndex}-${i}`}
              type="button"
              data-testid={`question-multi-option-${i}`}
              disabled={busy || sent}
              onClick={() => handleAnswer("select", i, undefined, mqIndex)}
              className="w-full rounded-lg bg-sky-500/10 px-2 py-1.5 text-left text-xs text-sky-800 hover:bg-sky-500/20 disabled:opacity-40 dark:bg-sky-400/10 dark:text-sky-200"
            >
              <span className="mr-1.5 rounded bg-sky-600 px-1 py-0.5 font-mono text-[10px] font-semibold text-white">
                {i + 1}
              </span>
              {o.label}
              {o.description !== "" && (
                <span className="ml-1 text-[10px] text-slate-500 dark:text-slate-400">
                  {o.description}
                </span>
              )}
            </button>
          ))}
        </div>
        {!sent && isLast && (
          <button
            type="button"
            data-testid="question-multi-submit"
            disabled={busy}
            onClick={() => handleAnswer("submit")}
            className="mt-2 rounded-full bg-sky-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-sky-700 disabled:opacity-40"
          >
            提交答案
          </button>
        )}
      </InteractiveCard>
    );
  }

  const q = questions[0];
  // 自由作答入口的**渲染条件**（丁T5 §2.4；复评 F6-3 收紧）：
  // - `info.freeText === true`（后端按**工具**判：只有 claude 定案）；
  // - **且题目形态是单选**（后端 `free_text_shape_supported`）——多选屏的自由作答行
  //   渲染为 `4. [ ] Type something`（带勾选框，实机截图
  //   `C-s8-cursor-submit-20260921-015844.png` 第 4 行），与「剥编号后以
  //   `Type something` 开头」的定位判据不符 → 后端已 409 拒绝。前端**同步不渲染**
  //   输入框（否则是给用户一个必然失败的按钮），改渲染「请在终端作答」引导。
  //
  // 判据来源说明：前端**不猜**这个限制，而是与后端同源——`multiSelect` 来自同一份
  // questions 载荷；`info.freeText` 由后端按工具给。两处判据合起来 = 后端的
  // `free_text_supported(tool) && free_text_shape_supported(q)`。
  const freeTextEnabled = info.freeText === true && !q.multiSelect;

  return (
    <InteractiveCard
      tone="question"
      testId="question-card"
      mode={q.multiSelect ? "multi" : "single"}
      title={q.multiSelect ? "等待回答（多选）" : "等待回答"}
      titleSuffix={
        q.header ? (
          <span
            data-testid="question-header"
            className={`rounded px-1.5 py-0.5 text-xs font-medium ${toneTokens("question").badge} ${toneTokens("question").title}`}
          >
            {q.header}
          </span>
        ) : null
      }
    >
      <p data-testid="question-text" className="mt-1 text-sm text-slate-800 dark:text-slate-200">
        {q.question}
      </p>
      {/* **进行中态**（丁T5 §2.3）——替代「已发送按键」：提交/自由作答的整条闭环是
          一次同步请求，期间显示「正在做什么」（段名文案），请求返回后本块消失。 */}
      {busy && inProgress !== null && (
        <p
          data-testid="question-progress"
          data-stage={inProgress}
          className="mt-1.5 text-xs font-medium text-sky-700 dark:text-sky-400"
        >
          <span className="mr-1 inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-sky-500 align-middle" />
          {QUESTION_STAGE_PROGRESS[inProgress]}
        </p>
      )}
      {error !== null && (
        <p data-testid="question-error" className="mt-1 text-xs text-rose-600 dark:text-rose-400">
          {/* 中止（阶段机）时把「卡在哪一段」放在原因之前——用户第一眼要知道停在哪 */}
          {abortedStage !== null && (
            <span data-testid="question-aborted-stage" className="font-medium">
              中止于「{QUESTION_STAGE_LABELS[abortedStage]}」段：
            </span>
          )}
          {error}
        </p>
      )}
      {abortedStage !== null && (
        <p
          data-testid="question-aborted-hint"
          className="mt-1 text-xs text-amber-700 dark:text-amber-400"
        >
          已停止投递后续按键——请到终端查看当前对话框状态后重试
        </p>
      )}
      {sent && (
        <div className="mt-1.5 text-xs font-medium text-emerald-600 dark:text-emerald-400">
          <p data-testid="question-sent">
            {/* 阶段机动作走完整条闭环 → 追加「已走完提交闭环」；单键动作与走完整条的
                都保留「已发送按键」这句主文案（既有用例与用户习惯都认它）。 */}
            已发送按键{verified !== null && "（已走完提交闭环）"}
          </p>
          {/* **终态回执核验**的三态（丁T5）：true 不额外提示；false/未核验要如实说 */}
          {verified === false && (
            <p
              data-testid="question-verified-unseen"
              className="mt-1 text-amber-700 dark:text-amber-400"
            >
              已按屏读完成提交，但未在屏上见到完成回执——请到终端确认结果
            </p>
          )}
        </div>
      )}
      {!sent && (
        <>
          <div className="mt-2 space-y-1.5">
            {q.options.map((o, i) => (
              <button
                key={`question-option-${i}`}
                type="button"
                data-testid={`question-option-${i}`}
                disabled={busy}
                onClick={() => handleAnswer(q.multiSelect ? "toggle" : "select", i)}
                className={`flex w-full items-start gap-2 rounded-lg px-2.5 py-1.5 text-left text-sm disabled:opacity-40 ${
                  q.multiSelect && checked.has(i)
                    ? "bg-sky-500/20 text-sky-800 dark:bg-sky-400/20 dark:text-sky-300"
                    : "bg-sky-500/5 text-slate-700 hover:bg-sky-500/10 dark:bg-sky-400/5 dark:text-slate-300 dark:hover:bg-sky-400/10"
                }`}
              >
                <span className="mt-0.5 inline-flex h-4 w-4 shrink-0 items-center justify-center rounded bg-sky-500/15 text-[10px] font-semibold text-sky-700 dark:bg-sky-400/15 dark:text-sky-400">
                  {i + 1}
                </span>
                <span className="min-w-0">
                  <span className="block font-medium">{o.label}</span>
                  {o.description && (
                    <span className="mt-0.5 block text-xs text-slate-500 dark:text-slate-400">
                      {o.description}
                    </span>
                  )}
                </span>
              </button>
            ))}
          </div>
          {q.multiSelect && (
            <button
              type="button"
              data-testid="question-submit"
              disabled={busy || checked.size === 0}
              onClick={() => handleAnswer("submit")}
              className="mt-2 w-full rounded-full bg-sky-600 px-3 py-1.5 text-sm font-medium text-white disabled:opacity-40 dark:bg-sky-500"
            >
              提交勾选
            </button>
          )}
          <button
            type="button"
            data-testid="question-cancel"
            disabled={busy}
            onClick={() => handleAnswer("cancel")}
            className="mt-1.5 w-full rounded-full bg-slate-500/10 px-3 py-1.5 text-sm text-slate-600 disabled:opacity-40 dark:bg-slate-400/10 dark:text-slate-300"
          >
            取消回答
          </button>
          {/* ===== 丁T5 §2.4：卡内自由作答输入框（入口 1；仅单题卡，本分支恒单题）===== */}
          {freeTextEnabled ? (
            <div className="mt-2" data-testid="question-freetext">
              <p
                data-testid="question-freetext-label"
                className="mb-1 text-xs text-slate-500 dark:text-slate-400"
              >
                或直接输入回答（将作为本题的答案发送到终端）
              </p>
              <div className="flex gap-1.5">
                <input
                  data-testid="question-freetext-input"
                  aria-label="回答内容"
                  type="text"
                  value={freeText}
                  maxLength={MAX_FREE_TEXT_CHARS}
                  disabled={busy}
                  onChange={(e) => setFreeText(e.target.value.slice(0, MAX_FREE_TEXT_CHARS))}
                  placeholder="输入你的回答…"
                  className="min-w-0 flex-1 rounded-lg border border-slate-200 px-2.5 py-1.5 text-sm text-slate-800 placeholder:text-slate-400 focus:ring-2 focus:ring-sky-500/40 focus:outline-none disabled:opacity-50 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-200"
                />
                <button
                  type="button"
                  data-testid="question-freetext-send"
                  disabled={busy || freeText.trim() === ""}
                  onClick={() => handleAnswer("freeText", undefined, freeText)}
                  className="shrink-0 rounded-full bg-sky-600 px-3 py-1.5 text-sm font-medium text-white disabled:opacity-40 dark:bg-sky-500"
                >
                  作为回答发送
                </button>
              </div>
            </div>
          ) : (
            <p
              data-testid="question-freeform-hint"
              className="mt-1.5 text-xs text-slate-500 dark:text-slate-400"
            >
              {/* 降级文案**说清是哪种限制**（两种成因用户动作相同——都去终端——但原因
                  不同，写清楚能少一次困惑）：① 多选题 → 「多选卡」限制（复评 F6-3，
                  题目形态维度）；② 工具未定案 → 「该工具尚未实测」（§2.8，工具维度）。 */}
              {q.multiSelect
                ? "需自由作答？多选题请到终端作答（多选屏的自由作答行形态与远程入口不匹配）"
                : "需自由作答？该工具的远程自由作答尚未实测，请在终端作答"}
            </p>
          )}
        </>
      )}
    </InteractiveCard>
  );
}
