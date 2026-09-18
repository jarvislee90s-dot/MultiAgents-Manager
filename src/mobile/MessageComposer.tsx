// 移动端会话输入区（M7 Task 7，W4）：挂在 SessionDetail 对话区之下（非预览分支）。
// - 可用性：挂载拉取一次 /session-send-info；不可注入 → 输入区禁用 + reason 展示；
//   拉取失败（网络异常等）静默不渲染——详情页正文照常（与 fetchSessionFiles
//   静默降级同一惯例），403 设备失效同语境（上层会回配对页）；
// - 发送：POST /session-send 三态回执 chip——delivered 绿「已送达终端」/ queued 黄
//   「排队中 第 N 位」+ [立即发送][撤回] / failed 红「发送失败：…」（重按发送即重试）；
// - 排队态 3s 轮询 /session-queue 刷新队位（unmount 清理定时器）；条目从队列消失
//   （已被 flush 送达 / 他端撤回）→ 回执收敛；
// - 多行原样上行（textarea 天然换行，不做回车发送），归一在服务端入队时一次完成。
import { useCallback, useEffect, useState } from "react";
import {
  ApiError,
  fetchQueue,
  fetchSendInfo,
  queueJump,
  queueRetract,
  sessionSend,
  type SendInfo,
} from "./api";

interface MessageComposerProps {
  /** 会话（本组件只消费 id；结构化类型，完整 Session 可直接传入） */
  session: { id: string };
}

/** 回执条状态（与 SendResult 对应 + 网络层 ApiError 归入 failed） */
type Receipt =
  | { kind: "delivered" }
  | { kind: "queued"; itemId: number; position: number }
  | { kind: "failed"; error: string }
  | null;

/** 排队态轮询间隔（毫秒）：有排队项时刷新队位（与看板轮询同量级） */
const QUEUE_POLL_MS = 3000;

export default function MessageComposer({ session }: MessageComposerProps) {
  // 可用性：sendInfo=null 且未就绪 → 不渲染（加载中 / 拉取失败 / 403）
  const [sendInfo, setSendInfo] = useState<SendInfo | null>(null);
  const [infoReady, setInfoReady] = useState(false);
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  // 插队/撤回进行中（与发送互斥，防连点）
  const [busy, setBusy] = useState(false);
  const [receipt, setReceipt] = useState<Receipt>(null);

  // 挂载拉取一次输入区可用性；任何失败静默保持隐藏
  useEffect(() => {
    let alive = true;
    setInfoReady(false);
    fetchSendInfo(session.id)
      .then((info) => {
        if (!alive) return;
        setSendInfo(info);
        setInfoReady(info !== null);
      })
      .catch(() => {
        if (alive) setInfoReady(false);
      });
    return () => {
      alive = false;
    };
  }, [session.id]);

  // 排队态 3s 轮询：只依赖 itemId（position 变化不重建定时器）；条目消失 → 收敛。
  // setReceipt 用函数式更新且未变时返回原引用，避免无谓重渲染
  const queuedItemId = receipt?.kind === "queued" ? receipt.itemId : null;
  useEffect(() => {
    if (queuedItemId === null) return;
    const iv = setInterval(() => {
      void fetchQueue(session.id)
        .then((items) => {
          setReceipt((prev) => {
            if (prev?.kind !== "queued" || prev.itemId !== queuedItemId) return prev;
            const mine = items.find((i) => i.id === queuedItemId);
            if (!mine) return null; // 已被 flush 送达 / 他端撤回 → 回执收敛
            return mine.position === prev.position
              ? prev
              : { kind: "queued", itemId: prev.itemId, position: mine.position };
          });
        })
        .catch(() => {
          /* 单次轮询失败忽略，下轮再试 */
        });
    }, QUEUE_POLL_MS);
    return () => clearInterval(iv);
  }, [queuedItemId, session.id]);

  const injectable = sendInfo !== null && sendInfo.injectable;

  const handleSend = useCallback(async () => {
    const body = text.trim();
    if (!body || sending || busy || sendInfo === null || !sendInfo.injectable) return;
    setSending(true);
    try {
      // 多行原样上行（trim 只用于判空，不改写正文——归一在服务端）
      const res = await sessionSend(session.id, text);
      if (res.status === "delivered") {
        setText("");
        setReceipt({ kind: "delivered" });
      } else if (res.status === "queued") {
        setText("");
        setReceipt({ kind: "queued", itemId: res.itemId, position: res.position });
      } else {
        setReceipt({ kind: "failed", error: res.error });
      }
    } catch (e) {
      if (e instanceof ApiError) {
        const reason = typeof e.data?.reason === "string" ? e.data.reason : null;
        setReceipt({ kind: "failed", error: reason ?? e.message });
      } else {
        setReceipt({ kind: "failed", error: String(e) });
      }
    } finally {
      setSending(false);
    }
  }, [text, sending, busy, sendInfo, session.id]);

  const handleJump = useCallback(async () => {
    if (receipt?.kind !== "queued" || busy) return;
    setBusy(true);
    try {
      const j = await queueJump(session.id, receipt.itemId);
      if (j.status === "delivered") {
        setReceipt({ kind: "delivered" });
      } else {
        setReceipt({ kind: "failed", error: j.error });
      }
    } catch (e) {
      setReceipt({ kind: "failed", error: e instanceof ApiError ? e.message : String(e) });
    } finally {
      setBusy(false);
    }
  }, [receipt, busy, session.id]);

  const handleRetract = useCallback(async () => {
    if (receipt?.kind !== "queued" || busy) return;
    const itemId = receipt.itemId;
    setBusy(true);
    try {
      await queueRetract(session.id, itemId);
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) {
        /* 404 not_found（已送达 / 他端撤回）：不作失败提示，走下方刷新自然收敛 */
      } else {
        setReceipt({ kind: "failed", error: "撤回失败，可重试" });
        setBusy(false);
        return;
      }
    }
    try {
      const items = await fetchQueue(session.id);
      const mine = items.find((i) => i.id === itemId);
      setReceipt(mine ? { kind: "queued", itemId, position: mine.position } : null);
    } catch {
      setReceipt(null);
    } finally {
      setBusy(false);
    }
  }, [receipt, busy, session.id]);

  // 拉取未就绪 / 失败 / 403：不渲染（详情页正文照常）
  if (!infoReady || sendInfo === null) return null;

  const canSend = injectable && !sending && !busy && text.trim().length > 0;

  return (
    <div
      data-testid="message-composer"
      className="shrink-0 border-t border-slate-200 px-3 py-2 dark:border-slate-800"
    >
      {/* 不可注入：输入区整体禁用，仅留 reason 展示（输入行保留占位但不可用） */}
      {!injectable && (
        <p
          data-testid="send-disabled-reason"
          className="mb-2 text-xs text-slate-500 dark:text-slate-400"
        >
          无法发送：{sendInfo.reason ?? "该会话不支持远程注入"}
          {sendInfo.reasonCode ? `（${sendInfo.reasonCode}）` : ""}
        </p>
      )}
      {receipt !== null && (
        <div className="mb-2 flex flex-wrap items-center gap-2">
          {receipt.kind === "delivered" && (
            <span
              data-testid="send-receipt-delivered"
              className="rounded-full bg-emerald-500/10 px-2 py-0.5 text-xs text-emerald-700 dark:bg-emerald-400/10 dark:text-emerald-400"
            >
              已送达终端
            </span>
          )}
          {receipt.kind === "queued" && (
            <>
              <span
                data-testid="send-receipt-queued"
                className="rounded-full bg-amber-500/10 px-2 py-0.5 text-xs text-amber-700 dark:bg-amber-400/10 dark:text-amber-400"
              >
                {`排队中 第${receipt.position}位`}
              </span>
              <button
                type="button"
                data-testid="queue-jump"
                disabled={busy}
                onClick={handleJump}
                className="rounded-full bg-amber-500/20 px-2 py-0.5 text-xs text-amber-700 disabled:opacity-40 dark:bg-amber-400/20 dark:text-amber-300"
              >
                立即发送
              </button>
              <button
                type="button"
                data-testid="queue-retract"
                disabled={busy}
                onClick={handleRetract}
                className="rounded-full bg-slate-200 px-2 py-0.5 text-xs text-slate-600 disabled:opacity-40 dark:bg-slate-800 dark:text-slate-300"
              >
                撤回
              </button>
            </>
          )}
          {receipt.kind === "failed" && (
            <span
              data-testid="send-receipt-failed"
              className="rounded-full bg-rose-500/10 px-2 py-0.5 text-xs text-rose-700 dark:bg-rose-400/10 dark:text-rose-400"
            >
              {`发送失败：${receipt.error}（可重试）`}
            </span>
          )}
        </div>
      )}
      <div className="flex items-end gap-2">
        <textarea
          data-testid="composer-input"
          aria-label="消息输入"
          value={text}
          onChange={(e) => setText(e.target.value)}
          rows={2}
          disabled={!injectable}
          placeholder={injectable ? "输入消息发送到终端…" : "该会话不支持远程注入"}
          className="min-h-0 flex-1 resize-none rounded-lg border border-slate-200 px-3 py-2 text-sm text-slate-800 placeholder:text-slate-400 focus:ring-2 focus:ring-sky-500/40 focus:outline-none disabled:opacity-50 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-200"
        />
        <button
          type="button"
          data-testid="composer-send"
          disabled={!canSend}
          onClick={handleSend}
          className="shrink-0 rounded-full bg-sky-500/10 px-4 py-2 text-sm text-sky-700 disabled:opacity-40 dark:bg-sky-400/10 dark:text-sky-300"
        >
          {sending ? "发送中…" : "发送"}
        </button>
      </div>
    </div>
  );
}
