// 移动端审批卡（M8 Task 12，红卡选项卡 UI）：挂在 SessionDetail 正文视图
// messageArea 上方（waiting 态；预览/分屏分支不挂——MessageComposer 同一挂载惯例）。
// - 选项可用性：挂载拉取一次 /session-approve-options；available=false（含拉取
//   失败 / 网络异常）→ 卡自身自隐（SessionDetail 无需感知选项可用性，详情页
//   正文照常——fetchSessionFiles 静默降级同一惯例）；
// - 红卡视觉：红色边框卡 + 标题「等待批准」（红点呼吸对齐看板 waiting 状态点）+
//   选项按钮横排（label 渲染；响应载荷只含 id/label，键位是投递层机密不外泄 UI）；
// - drift=true → 提示条「映射待实测确认，若提示不符请用普通发送」；
// - 应答：POST /session-approve——key_sent → 「已发送按键」终态（按钮禁用）；
//   failed{error} → 错误文案可重试（按钮保持可点，重按即重试）；ApiError（409/404
//   带 data.error）→ 分診中文文案：not_waiting→「会话不在等待状态」、no_mapping→
//   「该工具暂不支持审批应答，请用普通发送」、其余显示 message。
// 局限（本任务范围裁决）：mount 只拉一次选项，卡内不做轮询——红卡的出现/消失依赖
// 页面数据刷新（SSE 快照 → 详情页重挂/卸载）自然带动；SSE 驱动卡内 re-fetch 属
// Task 12 后优化，不在本任务范围。
import { useCallback, useEffect, useState } from "react";
import { ApiError, fetchApproveOptions, sessionApprove, type ApproveOptionsView } from "./api";

interface ApproveCardProps {
  /** 会话（本组件只消费 id；结构化类型，完整 Session 可直接传入） */
  session: { id: string };
}

export default function ApproveCard({ session }: ApproveCardProps) {
  // 选项可用性：ready=false（加载中 / available=false / 拉取失败）→ 不渲染
  const [options, setOptions] = useState<ApproveOptionsView | null>(null);
  const [ready, setReady] = useState(false);
  // 应答进行中（防连点）
  const [busy, setBusy] = useState(false);
  // 终态：按键已投递（key_sent）——按钮禁用 +「已发送按键」
  const [sent, setSent] = useState(false);
  // 失败文案（failed{error} 回执 / ApiError 分診）——非 null 展示，按钮保持可点（可重试）
  const [error, setError] = useState<string | null>(null);

  // 挂载拉取一次选项可用性；available=false / 任何失败 → 静默保持隐藏
  useEffect(() => {
    let alive = true;
    setReady(false);
    fetchApproveOptions(session.id)
      .then((v) => {
        if (!alive) return;
        setOptions(v);
        setReady(v.available);
      })
      .catch(() => {
        if (alive) setReady(false);
      });
    return () => {
      alive = false;
    };
  }, [session.id]);

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

  // available=false（含加载中 / 拉取失败）：不渲染（卡自身自隐）
  if (!ready || options === null || !options.available) return null;

  return (
    <div
      data-testid="approve-card"
      className="shrink-0 rounded-xl border-2 border-rose-500/60 bg-rose-500/5 px-3 py-2 dark:border-rose-400/60 dark:bg-rose-400/5"
    >
      <div className="flex items-center gap-1.5">
        <span className="inline-block h-2 w-2 animate-pulse rounded-full bg-rose-500" />
        <span className="text-sm font-semibold text-rose-700 dark:text-rose-400">等待批准</span>
      </div>
      {options.drift && (
        <p data-testid="approve-drift" className="mt-1 text-xs text-amber-700 dark:text-amber-400">
          映射待实测确认，若提示不符请用普通发送
        </p>
      )}
      {error !== null && (
        <p data-testid="approve-error" className="mt-1 text-xs text-rose-600 dark:text-rose-400">
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
      <div className="mt-2 flex gap-2">
        {options.options.map((o) => (
          <button
            key={o.id}
            type="button"
            data-testid={`approve-option-${o.id}`}
            disabled={busy || sent}
            onClick={() => handleAnswer(o.id)}
            className="flex-1 rounded-full bg-rose-500/10 px-3 py-1.5 text-sm text-rose-700 disabled:opacity-40 dark:bg-rose-400/10 dark:text-rose-300"
          >
            {o.label}
          </button>
        ))}
      </div>
    </div>
  );
}
