// 移动端问答卡（批次乙 T8，AskUserQuestion 问答卡 · claude 先行）：挂在
// SessionDetail 正文视图 / 分屏对话列（waiting 态；ApproveCard 同一挂载惯例，
// key 前缀 question-* 与 approve-*/composer-* 互异防 duplicate-key）。
// - 可用性：挂载拉取一次 /session-question；拉取失败 / 网络异常 → 静默自隐；
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
// - 自由文本：TUI 自动追加的 "N. Type something." 行不在 questions 载荷（探测档案
//   §2 UI 结构备注），v1 不做注入（K4-K7 定案：定位+文本+回车序列未纳入）——卡片
//   渲染引导文案指向下方输入框普通消息发送；
// - 多问题（questions.length>1）：**只读卡**——题干罗列 +「请在终端完成作答」引导，
//   零注入按钮（翻页键序未测，「结论不超证据」；后端 answer 端点同样拒绝）；
// - 应答分診：key_sent → 「已发送按键」终态（按钮禁用）；failed{error} → 错误文案
//   可重试；ApiError（409/400 带 data.error）→ 分診中文文案：no_question→「当前没有
//   待回答的问题」、multi_questions→「多个问题请回到终端完成作答」、bad_index→
//   「选项序号无效，请刷新后重试」、其余显示 message。
// 局限（ApproveCard 同款）：mount 只拉一次，卡内不做轮询——卡片的出现/消失依赖
// 页面数据刷新（SSE 快照 → 详情页重挂/卸载）自然带动。
import { useCallback, useEffect, useState } from "react";
import {
  ApiError,
  fetchSessionQuestion,
  sessionQuestionAnswer,
  type QuestionAnswerAction,
  type QuestionInfoView,
} from "./api";

interface QuestionCardProps {
  /** 会话（本组件只消费 id；结构化类型，完整 Session 可直接传入） */
  session: { id: string };
}

export default function QuestionCard({ session }: QuestionCardProps) {
  // 可用性：ready=false（加载中 / 拉取失败）→ 不渲染
  const [info, setInfo] = useState<QuestionInfoView | null>(null);
  const [ready, setReady] = useState(false);
  // 应答进行中（防连点）
  const [busy, setBusy] = useState(false);
  // 多选本地勾选态（仅在 toggle 成功回执后切换——端点拒绝时本地状态不漂移）
  const [checked, setChecked] = useState<Set<number>>(() => new Set());
  // 终态：按键序列已投递（key_sent）——按钮禁用 +「已发送按键」。**toggle 不算终态**
  // （多选点选后仍需「提交」，置终态会锁死提交钮）
  const [sent, setSent] = useState(false);
  // 失败文案（failed{error} 回执 / ApiError 分診）——非 null 展示，按钮保持可点
  const [error, setError] = useState<string | null>(null);

  // 挂载拉取一次；拉取失败 → 静默保持隐藏（ready 只表示「载荷已落地」）
  useEffect(() => {
    let alive = true;
    setReady(false);
    fetchSessionQuestion(session.id)
      .then((v) => {
        if (!alive) return;
        setInfo(v);
        setReady(true);
      })
      .catch(() => {
        if (alive) setReady(false);
      });
    return () => {
      alive = false;
    };
  }, [session.id]);

  const handleAnswer = useCallback(
    async (action: QuestionAnswerAction, index?: number) => {
      if (busy || sent) return;
      setBusy(true);
      setError(null);
      try {
        const res = await sessionQuestionAnswer(session.id, action, index);
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
          } else {
            // select / submit / cancel：终态（数字已提交 / 三段式已发 / 已取消）
            setSent(true);
          }
        } else {
          setError(res.error);
        }
      } catch (e) {
        if (e instanceof ApiError) {
          const code = typeof e.data?.error === "string" ? e.data.error : null;
          if (code === "no_question") {
            setError("当前没有待回答的问题");
          } else if (code === "multi_questions") {
            setError("多个问题请回到终端完成作答");
          } else if (code === "bad_index") {
            setError("选项序号无效，请刷新后重试");
          } else {
            setError(e.message);
          }
        } else {
          setError(String(e));
        }
      } finally {
        setBusy(false);
      }
    },
    [busy, sent, session.id]
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
      <div
        data-testid="question-card"
        data-mode="tool-readonly"
        className="shrink-0 rounded-xl border border-sky-500/50 bg-sky-500/5 px-3 py-2 dark:border-sky-400/50 dark:bg-sky-400/5"
      >
        <div className="flex items-center gap-1.5">
          <span className="inline-block h-2 w-2 rounded-full bg-sky-500" />
          <span className="text-sm font-semibold text-sky-700 dark:text-sky-400">
            等待回答
          </span>
        </div>
        {q0.header && (
          <div
            data-testid="question-header"
            className="mt-1.5 inline-block rounded bg-sky-500/10 px-1.5 py-0.5 text-[10px] font-medium text-sky-700 dark:bg-sky-400/10 dark:text-sky-400"
          >
            {q0.header}
          </div>
        )}
        <p
          data-testid="question-text"
          className="mt-1 text-sm text-slate-800 dark:text-slate-200"
        >
          {q0.question}
        </p>
        <ol className="mt-1.5 space-y-0.5">
          {q0.options.map((o, i) => (
            <li
              key={`question-ro-opt-${i}`}
              data-testid={`question-readonly-option-${i}`}
              className="text-xs text-slate-700 dark:text-slate-300"
            >
              <span className="mr-1 font-mono text-slate-500 dark:text-slate-400">
                {i + 1}.
              </span>
              {o.label}
              {o.description && (
                <span className="ml-1 text-slate-500 dark:text-slate-400">
                  — {o.description}
                </span>
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
      </div>
    );
  }

  // 多问题：只读卡（翻页键序未测——零注入按钮，引导终端作答）
  if (questions.length > 1) {
    return (
      <div
        data-testid="question-card"
        data-mode="readonly"
        className="shrink-0 rounded-xl border border-sky-500/50 bg-sky-500/5 px-3 py-2 dark:border-sky-400/50 dark:bg-sky-400/5"
      >
        <div className="flex items-center gap-1.5">
          <span className="inline-block h-2 w-2 rounded-full bg-sky-500" />
          <span className="text-sm font-semibold text-sky-700 dark:text-sky-400">
            有 {questions.length} 个问题等待回答
          </span>
        </div>
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
      </div>
    );
  }

  const q = questions[0];

  return (
    <div
      data-testid="question-card"
      data-mode={q.multiSelect ? "multi" : "single"}
      className="shrink-0 rounded-xl border-2 border-sky-500/60 bg-sky-500/5 px-3 py-2 dark:border-sky-400/60 dark:bg-sky-400/5"
    >
      <div className="flex items-center gap-1.5">
        <span className="inline-block h-2 w-2 animate-pulse rounded-full bg-sky-500" />
        <span className="text-sm font-semibold text-sky-700 dark:text-sky-400">
          {q.multiSelect ? "等待回答（多选）" : "等待回答"}
        </span>
        {q.header && (
          <span
            data-testid="question-header"
            className="rounded bg-sky-500/10 px-1.5 py-0.5 text-xs font-medium text-sky-700 dark:bg-sky-400/10 dark:text-sky-400"
          >
            {q.header}
          </span>
        )}
      </div>
      <p data-testid="question-text" className="mt-1 text-sm text-slate-800 dark:text-slate-200">
        {q.question}
      </p>
      {error !== null && (
        <p data-testid="question-error" className="mt-1 text-xs text-rose-600 dark:text-rose-400">
          {error}
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
          <p
            data-testid="question-freeform-hint"
            className="mt-1.5 text-xs text-slate-500 dark:text-slate-400"
          >
            需自由作答？请用下方输入框直接回复
          </p>
        </>
      )}
    </div>
  );
}
