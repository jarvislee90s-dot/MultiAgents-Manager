// 移动端会话输入区（M7 Task 7，W4）：挂在 SessionDetail 对话区之下（非预览分支）。
// - 可用性：挂载拉取一次 /session-send-info；不可注入 → 输入区禁用 + reason 展示；
//   拉取失败（网络异常等）静默不渲染——详情页正文照常（与 fetchSessionFiles
//   静默降级同一惯例），403 设备失效同语境（上层会回配对页）；
// - 发送：POST /session-send 三态回执 chip——delivered 绿「已送达终端」/ queued 黄
//   「排队中 第 N 位」+ [立即发送][撤回] / failed 红「发送失败：…」（重按发送即重试）；
//   await 全程另有「投递中…」chip（灰3：慢消费者长文投递可达分钟级，界面不空白，
//   完成后被结果 chip 覆盖）；正文上限与后端 MAX_SEND_CHARS 对齐（10000，双保险）；
// - 排队态 3s 轮询 /session-queue 刷新队位（unmount 清理定时器）；条目从队列消失
//   （已被 flush 送达 / 他端撤回）→ 回执收敛；position=0（并发消费窗口，后端明示
//   须容忍）→ 显示「排队中」不带位次数字；
// - 插队/撤回失败对账（M9R P2-7）：忙时失败 / 网络异常先 fetchQueue 复核——条目
//   仍在 pending → 恢复排队视图（刷新队位、按钮保留，可重试）；确认不在队 →
//   中性收敛文案（gone chip，评审裁决：忙时失败的守卫方正是正在投递的 flush 循环，
//   条目不在队大概率=已送达，落 failed「可重试」会诱发重复注入）；复核自身网络
//   失败 → 保守恢复排队视图（队位沿用旧值，3s 轮询随后自愈）——拿不到「真不在队」
//   的证据就不落终态，避免按钮丢失后排队条目在 UI 上失控；
// - 撤回按联合返回值分流（评审必须1）：{ok:true} 服务端确认已撤 → 免复核直接收敛；
//   200 {status:"failed"}（忙时拒收、条目仍在队）→ 复核——不得丢弃返回值直接复核，
//   否则「忙时 + 复核也失败」双失败会让条目实际在队而 UI 永久失控；
// - sending 期间插队/撤回按钮加闸（disabled=busy||sending，评审必须3）：与发送
//   回执的 last-write-wins 竞态防线（按钮可见不可点，保持「不失联」意图）；
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

/** 回执条状态（与 SendResult 对应 + 网络层 ApiError 归入 failed；
 *  gone = 中性收敛（评审裁决）：条目经复核确认已离开队列——大概率已被送达，
 *  不标失败红色、不带「（可重试）」，防止用户重发造成重复注入） */
type Receipt =
  | { kind: "delivered" }
  | { kind: "queued"; itemId: number; position: number }
  | { kind: "failed"; error: string }
  | { kind: "gone"; message: string }
  | null;

/** 排队态轮询间隔（毫秒）：有排队项时刷新队位（与看板轮询同量级） */
const QUEUE_POLL_MS = 3000;

/** 正文长度上限：与后端 MAX_SEND_CHARS 对齐（服务端超限 400 拒收，前端
 *  maxLength 截断是第一道防线，onChange slice 为 jsdom/旧内核兜底的双保险） */
const MAX_SEND_CHARS = 10000;

/** gone 收敛文案定稿（评审裁决，中性、不带「可重试」）：jump 条目可能已送达
 *  （守卫方 flush 循环刚把队首投出）；retract 只需告知不在队 */
const GONE_JUMP_MESSAGE = "条目已离开队列（可能已送达，可在会话内容中确认）";
const GONE_RETRACT_MESSAGE = "条目已不在队列";

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

  // P2-7 失败对账（插队/撤回共用）：失败后复核 /session-queue——
  // - 条目仍在 pending → 恢复排队视图（刷新队位，「立即发送/撤回」按钮保留可重试）；
  // - 确认不在队（已被消费 / 他端撤回）→ onGone 终态（jump/retract 均为中性 gone
  //   收敛文案，评审裁决：不落 failed「可重试」，防重复注入）；
  // - 复核自身网络失败 → 保守恢复排队视图（队位沿用旧值，3s 轮询随后自愈）——
  //   拿不到「真不在队」的证据就不落终态，避免按钮丢失后排队条目在 UI 上失控
  const reconcileQueued = useCallback(
    async (itemId: number, prevPosition: number, onGone: () => void) => {
      try {
        const items = await fetchQueue(session.id);
        const mine = items.find((i) => i.id === itemId);
        if (mine) {
          setReceipt({ kind: "queued", itemId, position: mine.position });
        } else {
          onGone();
        }
      } catch {
        setReceipt({ kind: "queued", itemId, position: prevPosition });
      }
    },
    [session.id]
  );

  const handleJump = useCallback(async () => {
    if (receipt?.kind !== "queued" || busy) return;
    const { itemId, position } = receipt;
    setBusy(true);
    try {
      const j = await queueJump(session.id, itemId);
      if (j.status === "delivered") {
        setReceipt({ kind: "delivered" });
      } else {
        // 200 failed 回执（忙时「投递进行中，请稍后重试」等）：先复核再定终态——
        // 忙时失败的守卫方正是正在投递队首的 flush 循环，条目不在队大概率=已送达，
        // 走中性 gone 收敛（评审裁决：不得落 failed「可重试」诱发重复注入）
        await reconcileQueued(itemId, position, () =>
          setReceipt({ kind: "gone", message: GONE_JUMP_MESSAGE })
        );
      }
    } catch {
      // 网络层 / 非 2xx（404 条目已不在队等）：同样复核，在队即恢复、不在队中性收敛
      await reconcileQueued(itemId, position, () =>
        setReceipt({ kind: "gone", message: GONE_JUMP_MESSAGE })
      );
    } finally {
      setBusy(false);
    }
  }, [receipt, busy, session.id, reconcileQueued]);

  const handleRetract = useCallback(async () => {
    if (receipt?.kind !== "queued" || busy) return;
    const itemId = receipt.itemId;
    const { position } = receipt;
    setBusy(true);
    try {
      const r = await queueRetract(session.id, itemId);
      if ("ok" in r) {
        /* 服务端确认已撤（{ok:true}）：免复核，直接收敛（评审必须1——撤回目的已达成） */
        setReceipt(null);
      } else {
        /* 忙时 200 {status:"failed"}：条目未被撤、仍在队 → 复核对账。不得丢弃联合
           返回值直接复核——否则「忙时 + 复核也失败」双失败时条目实际在队、UI 却
           永久失控（评审必须1核心场景；reconcileQueued 复核失败保守恢复兜底） */
        await reconcileQueued(itemId, position, () =>
          setReceipt({ kind: "gone", message: GONE_RETRACT_MESSAGE })
        );
      }
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) {
        /* 404 not_found（已送达 / 他端撤回）：条目已不在队 → 中性收敛（不作失败提示） */
        setReceipt({ kind: "gone", message: GONE_RETRACT_MESSAGE });
      } else {
        // 网络错：同款复核——条目仍在队 → 恢复排队视图可重试；确认不在队 → 中性收敛
        await reconcileQueued(itemId, position, () =>
          setReceipt({ kind: "gone", message: GONE_RETRACT_MESSAGE })
        );
      }
    } finally {
      setBusy(false);
    }
  }, [receipt, busy, session.id, reconcileQueued]);

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
      {/* 投递中（灰3）：send await 全程在场——慢消费者长文投递可达分钟级，
          期间不空白；与既有回执并存（排队态的撤回/插队按钮不因发送而失联），
          完成后被结果 chip 覆盖（sending 翻转 false 即消失） */}
      {sending && (
        <div className="mb-2 flex flex-wrap items-center gap-2">
          <span
            data-testid="send-receipt-delivering"
            className="rounded-full bg-sky-500/10 px-2 py-0.5 text-xs text-sky-700 dark:bg-sky-400/10 dark:text-sky-300"
          >
            <span className="mr-1 inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-sky-500 align-middle" />
            投递中…
            <span className="ml-1 text-slate-500 dark:text-slate-400">长文投递可能需要几分钟</span>
          </span>
        </div>
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
                {/* position=0：并发消费把条目刚投出的窗口值（后端明示前端须容忍），
                    显示口径 → 「排队中」不带位次数字，等下一轮轮询刷新为真实队位 */}
                {`排队中${receipt.position >= 1 ? ` 第${receipt.position}位` : ""}`}
              </span>
              <button
                type="button"
                data-testid="queue-jump"
                disabled={busy || sending}
                onClick={handleJump}
                className="rounded-full bg-amber-500/20 px-2 py-0.5 text-xs text-amber-700 disabled:opacity-40 dark:bg-amber-400/20 dark:text-amber-300"
              >
                立即发送
              </button>
              <button
                type="button"
                data-testid="queue-retract"
                disabled={busy || sending}
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
          {receipt.kind === "gone" && (
            // 中性收敛（评审裁决）：条目经复核确认已离开队列——大概率已送达，
            // 不标失败红色、不带「（可重试）」，防重复注入
            <span
              data-testid="send-receipt-gone"
              className="rounded-full bg-slate-200/70 px-2 py-0.5 text-xs text-slate-600 dark:bg-slate-700/60 dark:text-slate-300"
            >
              {receipt.message}
            </span>
          )}
        </div>
      )}
      <div className="flex items-end gap-2">
        <textarea
          data-testid="composer-input"
          aria-label="消息输入"
          value={text}
          maxLength={MAX_SEND_CHARS}
          onChange={(e) => setText(e.target.value.slice(0, MAX_SEND_CHARS))}
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
