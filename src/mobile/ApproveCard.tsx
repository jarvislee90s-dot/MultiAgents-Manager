// 移动端审批卡（M8 Task 12，红卡选项卡 UI）：挂在 SessionDetail 正文视图
// messageArea 上方（waiting 态；预览/分屏分支不挂——MessageComposer 同一挂载惯例）。
// - 选项可用性：挂载拉取一次 /session-approve-options；拉取失败 / 网络异常 →
//   静默自隐；available=false 且无 reason（非 Waiting / 无映射 / 未命中）→ 自隐
//   （SessionDetail 无需感知选项可用性，详情页正文照常——fetchSessionFiles 静默
//   降级同一惯例）；available=false 且带 reason（严格档）→ 渲染提示条见下；
// - 红卡视觉：红色边框卡 + 标题「等待批准」（红点呼吸对齐看板 waiting 状态点）+
//   选项按钮横排（label 渲染；响应载荷只含 id/label，键位是投递层机密不外泄 UI）；
// - drift=true → 提示条「映射待实测确认，若提示不符请用普通发送」；
// - 严格档（M9R）：available=false 且后端下发 reason（Task 10：键位未实测确认时
//   下发「键位待实测确认，请用普通发送」）→ 卡片只渲染提示条（reason 原文内联，
//   琥珀色弱化视觉、不带红卡脉冲）不渲染按键组——键位未实测防误发；
//   available=false 且无 reason（非 Waiting / 无映射 / 未命中）→ 卡自隐（原惯例）；
// - 应答：POST /session-approve——key_sent → 「已发送按键」终态（按钮禁用）；
//   failed{error} → 错误文案可重试（按钮保持可点，重按即重试）；ApiError（409/404
//   带 data.error）→ 分診中文文案：not_waiting→「会话不在等待状态」、no_mapping→
//   「该工具暂不支持审批应答，请用普通发送」、no_session→「会话已结束，请返回看板刷新」、
//   其余显示 message。
// 局限（本任务范围裁决）：mount 只拉一次选项，卡内不做轮询——红卡的出现/消失依赖
// 页面数据刷新（SSE 快照 → 详情页重挂/卸载）自然带动；SSE 驱动卡内 re-fetch 属
// Task 12 后优化，不在本任务范围。
import { useCallback, useEffect, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import InteractiveCard, { toneTokens } from "./InteractiveCard";
import { ApiError, fetchApproveOptions, sessionApprove, type ApproveOptionsView } from "./api";

interface ApproveCardProps {
  /** 会话（本组件只消费 id；结构化类型，完整 Session 可直接传入） */
  session: { id: string };
}

/** 计划待确认条的脚注文案（丁T2）：预期态在场但后端没读到终端对话框选项——
 *  与后端 `remote/api.rs` 的降级语义同源（终端对话框可能尚未绘制/已关闭/不在
 *  Windows 可见窗口）。前端自持文案：后端在 available=true 形态下不下发 reason
 *  （reason 是 available=false 的严格档通道），故此处由前端给同义提示。 */
const PLAN_CHECK_MISS_HINT =
  "未读到终端对话框选项——请再点一次「检查终端对话框」，或直接在终端处理该确认";

export default function ApproveCard({ session }: ApproveCardProps) {
  // 选项可用性：ready=false（加载中 / 拉取失败）→ 不渲染（available/reason 分诊在渲染侧）
  const [options, setOptions] = useState<ApproveOptionsView | null>(null);
  const [ready, setReady] = useState(false);
  // 应答进行中（防连点）
  const [busy, setBusy] = useState(false);
  // 终态：按键已投递（key_sent）——按钮禁用 +「已发送按键」
  const [sent, setSent] = useState(false);
  // 失败文案（failed{error} 回执 / ApiError 分診）——非 null 展示，按钮保持可点（可重试）
  const [error, setError] = useState<string | null>(null);
  // 丁T2：「检查终端对话框」进行中（防连点）+ 检查后仍未读到选项的降级提示
  const [checking, setChecking] = useState(false);
  const [checkMissed, setCheckMissed] = useState(false);

  // 挂载拉取一次选项可用性；拉取失败 → 静默保持隐藏。
  // ready 只表示「载荷已落地」（available/reason 的分诊移到渲染侧——严格档
  // available=false + reason 也要渲染提示条，不能拿 available 当 ready）
  useEffect(() => {
    let alive = true;
    setReady(false);
    fetchApproveOptions(session.id)
      .then((v) => {
        if (!alive) return;
        setOptions(v);
        setReady(true);
      })
      .catch(() => {
        if (alive) setReady(false);
      });
    return () => {
      alive = false;
    };
  }, [session.id]);

  /** 「检查终端对话框」（丁T2）：重拉一次选项端点——后端在计划预期态下**现场屏读**，
   *  命中即回 N 选项（id=`dialog:<n>`），未命中回零选项 + planPending=true。
   *  与挂载那次拉取同一条数据通道（无新端点）；失败静默保留原载荷 + 显示降级提示。 */
  const handleCheck = useCallback(async () => {
    if (checking) return;
    setChecking(true);
    setCheckMissed(false);
    try {
      const v = await fetchApproveOptions(session.id);
      setOptions(v);
      // 仍未读到选项（planPending 仍立且无 dialog）→ 明示未命中，不假装成功
      const stillPending = v.available === true && v.planPending === true && v.dialog !== true;
      setCheckMissed(stillPending && v.options.length === 0);
    } catch {
      setCheckMissed(true);
    } finally {
      setChecking(false);
    }
  }, [checking, session.id]);

  const handleAnswer = useCallback(
    async (optionId: string) => {
      if (busy || sent) return;
      setBusy(true);
      setError(null);
      try {
        const res = await sessionApprove(session.id, optionId);
        if (res.status === "key_sent") {
          setSent(true);
        } else {
          setError(res.error);
        }
      } catch (e) {
        if (e instanceof ApiError) {
          const code = typeof e.data?.error === "string" ? e.data.error : null;
          if (code === "not_waiting") {
            setError("会话不在等待状态");
          } else if (code === "no_mapping") {
            setError("该工具暂不支持审批应答，请用普通发送");
          } else if (code === "no_session") {
            setError("会话已结束，请返回看板刷新");
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

  // 加载中 / 拉取失败 / options 未落地：不渲染
  if (!ready || options === null) return null;
  // 严格档（M9R）：available=false 且后端给出 reason → 只渲染提示条不渲染按键；
  // available=false 且无 reason（非 Waiting / 无映射 / 未命中）：卡自身自隐（原惯例）
  const hintOnly =
    !options.available && typeof options.reason === "string" && options.reason.trim() !== "";
  if (!options.available && !hintOnly) return null;

  // 严格档提示条模式：reason 原文内联（后端中文），琥珀色弱化（非红卡脉冲——
  // 无可操作按键，避免误导），不渲染按键组
  if (hintOnly) {
    return (
      <div
        data-testid="approve-card"
        data-mode="hint"
        className="shrink-0 rounded-xl border border-amber-500/50 bg-amber-500/5 px-3 py-2 dark:border-amber-400/50 dark:bg-amber-400/5"
      >
        <p data-testid="approve-hint" className="text-xs text-amber-700 dark:text-amber-400">
          {options.reason}
        </p>
      </div>
    );
  }

  // ===== 丁T2：计划待确认条（codex/kimi 的计划确认框）=====
  //
  // 形态：`available=true ∧ planPending=true ∧ dialog≠true`（零选项是**预期**，不是错误）。
  // 为什么单独一条渲染分支（而不是复用选项卡）：
  // - 此形态下后端**刻意不下发**映射表键位（codex 的 y/esc 是补丁审批键位、kimi 的
  //   数字通道已被 R1-1 证伪不可依赖）——没有可渲染的按钮，选项卡会渲染成空组；
  // - 用户的下一步动作是**去终端看**（可能对话框没绘制/已关闭）或**点检查重试**
  //   （对话框刚绘制出来时，屏读一次就能拿到 N 选项）。
  //
  // 计划正文照常渲染（T8 聚合机制，`plan` 字段）——用户点检查前先看到计划全文。
  const planPendingOnly =
    options.available && options.planPending === true && options.dialog !== true;
  if (planPendingOnly) {
    return (
      <InteractiveCard
        tone="approve"
        testId="approve-card"
        mode="plan-pending"
        title="计划待确认"
        footer={
          <>
            <button
              type="button"
              data-testid="approve-plan-check"
              disabled={checking}
              onClick={handleCheck}
              className="mt-2 rounded-full bg-rose-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-rose-700 disabled:opacity-40"
            >
              {checking ? "检查中…" : "检查终端对话框"}
            </button>
            {/* 检查未命中：明示（红线 3——不假装成功）；命中则本条整个消失（走选项卡分支） */}
            {checkMissed && (
              <p
                data-testid="approve-plan-check-miss"
                className="mt-1 text-xs text-amber-700 dark:text-amber-400"
              >
                {PLAN_CHECK_MISS_HINT}
              </p>
            )}
          </>
        }
      >
        {/* 计划待确认条：**检查未命中即清除**（任务书语义「预期态清除：下一个用户消息
            注入或检查未命中时清除」）——点过检查且屏读仍没读到选项时，本条不再显示
            （不再断言「终端正在等这个计划」——那是未经验证的声明，§2.8 不假装），
            改为下方脚注的「未读到」明示 + 检查钮可再试。用户注入下一条消息后，
            消息尾部判据（`isPlanPending`）会让整张卡不再挂载 = 预期态彻底清除。 */}
        {!checkMissed && (
          <p
            data-testid="approve-plan-pending"
            className="mt-1 text-xs text-rose-700/80 dark:text-rose-400/80"
          >
            终端正在等待这个计划的确认——请到终端对话框选择，或点下方按钮读取选项
          </p>
        )}
        {/* 计划全文（T8 聚合；无计划消息 → 不渲染主体） */}
        {options.plan != null && options.plan.content.trim() !== "" && (
          <div
            data-testid="approve-plan"
            data-plan-file={options.plan.isFile ? "true" : "false"}
            className="mt-2 max-h-64 overflow-y-auto rounded-lg border border-rose-500/30 bg-white/60 p-2 text-xs text-slate-800 dark:border-rose-400/30 dark:bg-slate-900/60 dark:text-slate-200"
          >
            <p className="mb-1 text-[11px] font-medium tracking-wide text-rose-700/80 uppercase dark:text-rose-400/80">
              {options.plan.isFile ? "计划文件" : "计划内容"}
            </p>
            {options.plan.isFile ? (
              <p data-testid="approve-plan-file" className="font-mono break-all">
                {options.plan.content}
              </p>
            ) : (
              <div className="prose-sm max-w-none">
                <ReactMarkdown remarkPlugins={[remarkGfm]}>{options.plan.content}</ReactMarkdown>
              </div>
            )}
          </div>
        )}
      </InteractiveCard>
    );
  }

  return (
    <InteractiveCard
      tone="approve"
      testId="approve-card"
      mode={options.dialog ? "dialog" : "binary"}
      title={options.dialog ? "等待批准（终端对话框）" : "等待批准"}
      footer={
        <>
          {error !== null && (
            <p
              data-testid="approve-error"
              className="mt-1 text-xs text-rose-600 dark:text-rose-400"
            >
              {error}
            </p>
          )}
          {sent && (
            <p
              data-testid="approve-sent"
              className="mt-1.5 text-xs font-medium text-emerald-600 dark:text-emerald-400"
            >
              已发送按键
            </p>
          )}
        </>
      }
      actions={
        <div className={options.dialog ? "space-y-1" : "flex gap-2"}>
          {/* T5：对话框选项 → 纵向编号列表（真实选项文本较长，纵向排布可读；
              编号徽标 = 将注入的数字键，用户所见即所按）；二元项维持既有横排 */}
          {options.dialog
            ? options.options.map((o, i) => (
                <button
                  key={o.id}
                  type="button"
                  data-testid={`approve-option-${o.id}`}
                  disabled={busy || sent}
                  onClick={() => handleAnswer(o.id)}
                  className={`flex w-full items-start gap-2 rounded-lg px-2 py-1.5 text-left text-xs disabled:opacity-40 ${toneTokens("approve").action}`}
                >
                  <span
                    className={`shrink-0 rounded px-1.5 py-0.5 font-mono text-[10px] font-semibold ${toneTokens("approve").badge}`}
                  >
                    {i + 1}
                  </span>
                  <span className="min-w-0 flex-1 break-words">{o.label}</span>
                </button>
              ))
            : options.options.map((o) => (
                <button
                  key={o.id}
                  type="button"
                  data-testid={`approve-option-${o.id}`}
                  disabled={busy || sent}
                  onClick={() => handleAnswer(o.id)}
                  className={`flex-1 rounded-full px-3 py-1.5 text-sm disabled:opacity-40 ${toneTokens("approve").action}`}
                >
                  {o.label}
                </button>
              ))}
        </div>
      }
    >
      {options.dialog && (
        <p
          data-testid="approve-dialog-label"
          className="mt-1 text-xs text-rose-700/80 dark:text-rose-400/80"
        >
          以下选项读自终端对话框，点按即代你按对应数字键
        </p>
      )}
      {/* T8：审批点 plan 聚合——计划确认类审批卡主体即见计划全文（不再要用户去
          消息流翻）。markdown 直出（claude/codex 的 kind="plan"）；kimi 的
          kind="plan-file" 是**文件路径**，此处以路径提示呈现（全文走文件预览，
          与消息流口径一致，不重复读文件正文）。无计划（plan null）→ 不渲染 */}
      {options.plan != null && options.plan.content.trim() !== "" && (
        <div
          data-testid="approve-plan"
          data-plan-file={options.plan.isFile ? "true" : "false"}
          className="mt-2 max-h-64 overflow-y-auto rounded-lg border border-rose-500/30 bg-white/60 p-2 text-xs text-slate-800 dark:border-rose-400/30 dark:bg-slate-900/60 dark:text-slate-200"
        >
          <p className="mb-1 text-[11px] font-medium tracking-wide text-rose-700/80 uppercase dark:text-rose-400/80">
            {options.plan.isFile ? "计划文件" : "计划内容"}
          </p>
          {options.plan.isFile ? (
            <p data-testid="approve-plan-file" className="font-mono break-all">
              {options.plan.content}
            </p>
          ) : (
            <div className="prose-sm max-w-none">
              <ReactMarkdown remarkPlugins={[remarkGfm]}>{options.plan.content}</ReactMarkdown>
            </div>
          )}
        </div>
      )}
      {options.drift && (
        <p data-testid="approve-drift" className="mt-1 text-xs text-amber-700 dark:text-amber-400">
          映射待实测确认，若提示不符请用普通发送
        </p>
      )}
      {/* R1-3 降级警示（计划红线 3）：命中审批但未读到终端对话框选项 → 终端可能正
          显示多选项而二元键可能错位，必须显式提示用户去终端核对（后端下发文案原文） */}
      {typeof options.degradedHint === "string" && options.degradedHint.trim() !== "" && (
        <p
          data-testid="approve-degraded-hint"
          className="mt-1 text-xs text-amber-700 dark:text-amber-400"
        >
          {options.degradedHint}
        </p>
      )}
    </InteractiveCard>
  );
}
