// 移动端会话输入区（M7 Task 7，W4）：挂在 SessionDetail 对话区之下（非预览分支）。
// - 可用性：挂载拉取一次 /session-send-info；不可注入 → 输入区禁用 + reason 展示；
//   拉取失败（网络异常等）静默不渲染——详情页正文照常（与 fetchSessionFiles
//   静默降级同一惯例），403 设备失效同语境（上层会回配对页）；
// - 发送：POST /session-send 回执 chip——delivered 绿「已送达终端」/ submitted 灰
//   「已投递至终端输入，agent 空闲后处理（未确认落盘）」（D7/T3 中性回执，验收
//   问题 #5：注入 Ok + 戳未中 + 屏读无滞留草稿 = 已被 TUI 收进内部队列，非失败
//   **不提供重试**，重试 = 双发且 TUI 那份无法撤回）/ queued 黄
//   「排队中 第 N 位」+ [立即发送][撤回] / failed 红「发送失败：…」（重按发送即重试）；
//   await 全程另有「投递中…」chip（灰3：慢消费者长文投递可达分钟级，界面不空白，
//   完成后被结果 chip 覆盖）；正文上限与后端 MAX_SEND_CHARS 对齐（10000，双保险）；
// - 多列队列表（D8，验收问题 #6）：完整渲染 /session-queue——挂载拉一次（进入会话
//   即见桌面端/他端排的既有队列）+ 复用 3s 轮询通道（有排队回执或列表非空即持续，
//   列表变空即停）；逐条 = 预览（去 [mobile 设备名] 前缀、附件标记替换 [附件]、
//   截前 12 字符）+ 位次（position=0 容忍口径不变）+ 立即发送/修改/撤回三钮（按条
//   id 走既有端点，不限本端发送的条目）；底部「空闲时将按序自动发送」说明。
//   「排队中 第 N 位」chip 从回执槽移除（由列表取代）；queued 回执仍内部存在——
//   发送后到列表刷新之间的即时反馈靠行乐观 upsert，失败对账（reconcileQueued）
//   与 3s 轮询仍以它驱动；单回执槽只保留最近一次发送的送达/中性/失败终态；
// - 排队列 3s 轮询 /session-queue（unmount 清理定时器）；我方条目从队列消失
//   （已被 flush 送达 / 他端撤回）→ 回执收敛留痕 + 行随权威列表移除；
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
// - 修改重发只入队（D6，验收问题 #4）：「修改」确认出队后置 queueOnlyNext 标志，
//   下一次发送携带 queueOnly=true 强制入队——防「文件说闲、TUI 实忙」窗口把
//   修改后的重发直发出去（变相插队）；入队后的放行节奏：会话转闲跃迁后事件臂
//   即时放行，已空闲且无跃迁时由 60s 周期兜底放行（可达分钟级），行为收敛；
// - 多行原样上行（textarea 天然换行，不做回车发送），归一在服务端入队时一次完成。
import { useCallback, useEffect, useRef, useState } from "react";
import type { ClipboardEvent as ReactClipboardEvent } from "react";
import { CircleHelp, Plus } from "lucide-react";
import {
  ApiError,
  fetchQueue,
  fetchSendInfo,
  queueJump,
  queueRetract,
  sessionSend,
  uploadAttachment,
  type QueueItemView,
  type SendInfo,
} from "./api";

interface MessageComposerProps {
  /** 会话（本组件只消费 id；结构化类型，完整 Session 可直接传入） */
  session: { id: string };
}

/** 回执条状态（与 SendResult 对应 + 网络层 ApiError 归入 failed；
 *  gone = 中性收敛（评审裁决）：条目经复核确认已离开队列——大概率已被送达，
 *  不标失败红色、不带「（可重试）」，防止用户重发造成重复注入） */
/** 待发附件条目（组件内态）：status=uploading → ready/failed；
 *  path = 服务端落盘后的绝对路径（仅 ready 有） */
type PendingAttachment = {
  id: string;
  name: string;
  isImage: boolean;
  status: "uploading" | "ready" | "failed";
  path?: string;
  error?: string;
};

type Receipt =
  | { kind: "delivered" }
  | {
      /** D7/T3 中性回执（验收问题 #5）：已投递未确认——消息已被 TUI 收进内部
       *  队列，非失败、无重试入口（重试 = 双发且 TUI 那份无法撤回） */
      kind: "submitted";
    }
  | {
      kind: "queued";
      itemId: number;
      position: number;
      /** 排队正文（2026-09-20「修改」按钮）：发送时就在手上，随回执携带——
       *  「修改」确认出队后据此放回输入框，零额外请求。条目内容在队内不可变 */
      content: string;
    }
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
/** 轮询发现条目消失（2026-09-20）：此前静默清空回执——用户实测「桌面端插队后，
 *  手机端排队提示无声消失」。队列会话级共享、无归属标记，移动端无法区分被送达
 *  还是被其他端处理，故给中性提示留痕；真实归属需后端审计回传，边缘场景不做 */
const GONE_POLL_CONSUMED_MESSAGE = "该消息已不在队列（可能已送达，或由电脑端处理）";

/** D8 队列预览截断长度（字符）：逐条预览只给「前十余字符」一眼可辨的量 */
const PREVIEW_MAX_CHARS = 12;
/** W1 来源标记前缀（服务端 compose_injection 注入，非用户正文）：预览与「修改」
 *  放回正文时剥离——不剥则修改重发会二次叠加前缀 */
const MOBILE_PREFIX_RE = /^\[mobile[^\]]*\]\s*/;
/** 附件内联标记（文件池既有约定）：<image|file path="…"> → 预览中替换为 [附件] */
const ATTACH_MARKUP_RE = /<(?:image|file)\s+path="[^"]*">/g;

/** 剥离 [mobile 设备名] 前缀（正则不命中则原样返回）：预览与修改放回共用 */
function stripMobilePrefix(content: string): string {
  return content.replace(MOBILE_PREFIX_RE, "");
}

/** 队列条目预览（D8）：去 [mobile …] 前缀 → 附件标记替换 [附件] → 截前 12 字符
 *  （不足则全显）。仅用于展示；「修改」放回正文只剥前缀、保留附件标记行 */
function queueItemPreview(content: string): string {
  const body = stripMobilePrefix(content).replace(ATTACH_MARKUP_RE, "[附件]");
  return body.length > PREVIEW_MAX_CHARS ? `${body.slice(0, PREVIEW_MAX_CHARS)}…` : body;
}

/** 行按 id upsert（乐观插入/更新共用的收敛入口）：命中替换、未命中尾部追加——
 *  发送入队的即时反馈与复核保守恢复都经它把行放回列表，3s 轮询随后以服务端
 *  权威列表整表覆盖 */
function upsertQueueItem(items: QueueItemView[], item: QueueItemView): QueueItemView[] {
  const idx = items.findIndex((i) => i.id === item.id);
  if (idx < 0) return [...items, item];
  const next = items.slice();
  next[idx] = item;
  return next;
}

export default function MessageComposer({ session }: MessageComposerProps) {
  // 可用性：sendInfo=null 且未就绪 → 不渲染（加载中 / 拉取失败 / 403）
  const [sendInfo, setSendInfo] = useState<SendInfo | null>(null);
  const [infoReady, setInfoReady] = useState(false);
  const [text, setText] = useState("");
  /** 输入框 ref：「修改」确认出队后把正文放回输入框时聚焦（移动端直接可改） */
  const inputRef = useRef<HTMLTextAreaElement>(null);
  /** 在途上传的中断器（id → controller）：上传中移除 chip 时中断 fetch */
  const uploadCtrlsRef = useRef(new Map<string, AbortController>());
  /** 隐式文件选择器 ref：「+」钮 click 转发 */
  const fileInputRef = useRef<HTMLInputElement>(null);
  /** 待发附件（2026-09-20）：uploading → ready（含落盘绝对路径）/ failed；
   *  发送时仅 ready 的拼内联标记行，failed 不上送（用户裁决：不做通用美化，
   *  附件路径是给 agent 读的） */
  const [attachments, setAttachments] = useState<PendingAttachment[]>([]);
  /** 附件「?」说明展开态（2026-09-20 知情披露）：点开才显示，不平铺常驻 */
  const [attachHintOpen, setAttachHintOpen] = useState(false);
  /** 会话无项目目录（服务端 404 no_cwd 一次即知）：禁用上传钮（与 R5 禁用口径同源） */
  const [noCwd, setNoCwd] = useState(false);
  const [sending, setSending] = useState(false);
  // 插队/撤回进行中（与发送互斥，防连点）
  const [busy, setBusy] = useState(false);
  const [receipt, setReceipt] = useState<Receipt>(null);
  /** D8 完整队列表（/session-queue 的 items）：逐条渲染的数据源——不限本端发送
   *  的条目（桌面端/他端排的队同样可见可操作）。乐观行（发送即时反馈/复核保守
   *  恢复）与权威列表（挂载拉取/3s 轮询）都收敛到这里 */
  const [queueItems, setQueueItems] = useState<QueueItemView[]>([]);
  /** 本地队列认知最近一次翻转的时刻（T4 复评 I1/M1 时序防御）：发送乐观入队、
   *  插队送达、撤回确认/404、复核三分支任一落地即前移。tick 快照若**发起**早于
   *  该时刻，说明它反映的是翻转前的服务端视图——既不可整表覆盖（会抹乐观行 /
   *  复活已确认出队的幽灵行），也不可据其判 gone（会把「刚入队」误判成已送达，
   *  唯一条目时更会停摆轮询 → 用户重发 = 重复注入，本项目头号禁忌）。作废一份
   *  快照的代价 ≤ 3s 自愈，方向一律保守（同毫秒按更旧处理，用 <=） */
  const queueMutatedAtRef = useRef(0);
  /** 修改重发只入队标志（D6）：「修改」确认出队后置 true，下一次发送携带
   *  queueOnly=true 并清除（消费即清——失败重试不再带标志，回归普通发送语义）。
   *  保守语义：修改后的重发一律入队，用户手动改字不清除标志。入队后的放行节奏：
   *  会话转闲跃迁后事件臂即时放行；已空闲且无跃迁时由 60s 周期兜底放行（可达
   *  分钟级）。队列存储不携带该标志（后端仅影响入队决策，flush 循环对
   *  queueOnly 项与普通队列项同权） */
  const [queueOnlyNext, setQueueOnlyNext] = useState(false);

  // 挂载拉取一次输入区可用性；任何失败静默保持隐藏
  useEffect(() => {
    let alive = true;
    setInfoReady(false);
    // 换会话即弃修改重发标志（D6）：queueOnlyNext 是上一个会话「修改」的遗愿，
    // 不得泄漏到新会话的首发（保守方向虽无害，语义上仍属错位）
    setQueueOnlyNext(false);
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

  // D8：挂载/换会话拉一次完整队列——进入会话即见既有队列（桌面端/他端排的队也要
  // 能看到）；换会话先清列表防串台。拉取失败静默（列表留空），排队回执在场的 3s
  // 轮询通道随后自愈
  useEffect(() => {
    let alive = true;
    setQueueItems([]);
    fetchQueue(session.id)
      .then((items) => {
        if (alive) setQueueItems(items);
      })
      .catch(() => {
        /* 挂载拉取失败静默：留空待轮询/复核自愈 */
      });
    return () => {
      alive = false;
    };
  }, [session.id]);

  // D8 排队列 3s 轮询（复用既有 3s 通道，不另起轮询）：有排队回执 **或** 列表非空
  // 即持续；列表变空（且无排队回执）→ 停轮询。依赖是布尔量而非列表本体——位次
  // 变化/整表覆盖不重建定时器。setReceipt 用函数式更新且未变时返回原引用
  const queueTracked = receipt?.kind === "queued" || queueItems.length > 0;
  useEffect(() => {
    if (!queueTracked) return;
    // I2：换会话/停轮询触发 effect 重建时置 false——此前会话的在途 tick 响应
    // 一律作废（旧会话的行不得经无条件 setQueueItems 落入新会话，与挂载 effect
    // 的 alive 守卫同款）
    let alive = true;
    const iv = setInterval(() => {
      // I1：先记快照发起时刻，再发 GET——响应到货时与本地认知翻转时刻比对
      const issuedAt = Date.now();
      void fetchQueue(session.id)
        .then((items) => {
          if (!alive) return; // I2：换会话/停轮询后的在途响应作废
          if (issuedAt <= queueMutatedAtRef.current) return; // I1/M1：早于本地认知翻转的旧快照整份作废（防御见 ref 声明处）
          setQueueItems(items);
          setReceipt((prev) => {
            if (prev?.kind !== "queued") return prev;
            const mine = items.find((i) => i.id === prev.itemId);
            if (!mine) {
              // 已被 flush 送达 / 他端插队 / 他端撤回 → 回执收敛（2026-09-20 前
              // 是静默 return null，用户实测困惑；队列会话级共享、无归属标记，
              // 移动端分不清谁触发，故落中性提示留痕）；行随权威列表一并移除
              return { kind: "gone", message: GONE_POLL_CONSUMED_MESSAGE };
            }
            return mine.position === prev.position
              ? prev
              : {
                  ...prev,
                  position: mine.position,
                };
          });
        })
        .catch(() => {
          /* 单次轮询失败忽略，下轮再试 */
        });
    }, QUEUE_POLL_MS);
    return () => {
      alive = false;
      clearInterval(iv);
    };
  }, [queueTracked, session.id]);

  const injectable = sendInfo !== null && sendInfo.injectable;

  const handleSend = useCallback(async () => {
    const body = text.trim();
    if (!body || sending || busy || sendInfo === null || !sendInfo.injectable) return;
    if (attachments.some((a) => a.status === "uploading")) return; // 上传中禁发（防消息先于落盘）
    setSending(true);
    try {
      // 附件标记行（文件池既有约定 <image|file path>）：拼在正文之后随消息注入，
      // agent 据路径读文件；failed 附件不拼（未落盘，拼了 agent 也读不到）
      const markup = attachments
        .filter((a) => a.status === "ready" && a.path)
        .map((a) => (a.isImage ? `<image path="${a.path}">` : `<file path="${a.path}">`))
        .join("\n");
      const fullText = markup ? `${text}\n${markup}` : text;
      // D6：修改重发的下一次发送带 queueOnly=true 强制入队，消费即清——本次发送
      // 失败的话，用户重试走的是普通发送语义（后端失败行已退出 pending，可重发）
      const forceQueue = queueOnlyNext;
      setQueueOnlyNext(false);
      // 多行原样上行（trim 只用于判空，不改写正文——归一在服务端）
      const res = await sessionSend(session.id, fullText, forceQueue ? true : undefined);
      if (res.status === "delivered") {
        setText("");
        setAttachments([]);
        setReceipt({ kind: "delivered" });
      } else if (res.status === "submitted") {
        // D7/T3 中性回执：注入 Ok + 戳未中 + 屏读无滞留草稿 = 已被 TUI 收进内部
        // 队列。消息已离开前端 → 输入框/附件清空对齐 delivered 口径；不提供重试
        // （TUI 那份无法撤回，重按发送 = 双发——可重试语义仅属于 failed 态）
        setText("");
        setAttachments([]);
        setReceipt({ kind: "submitted" });
      } else if (res.status === "queued") {
        setText("");
        setAttachments([]);
        // I1：本地「该条在队」认知自此成立——此前发起的在途 tick 快照可能不含
        // 本条，不得据此判 gone / 整表覆盖（时序防御见 queueMutatedAtRef 声明处）
        queueMutatedAtRef.current = Date.now();
        setReceipt({
          kind: "queued",
          itemId: res.itemId,
          position: res.position,
          content: fullText,
        });
        // D8 即时反馈：行乐观 upsert（发送后到列表刷新之间不上墙等 3s）；content
        // 用本端原文（此时服务端 compose 后的带前缀文本尚未回拉，3s 轮询整表覆盖）
        setQueueItems((prev) =>
          upsertQueueItem(prev, {
            id: res.itemId,
            content: fullText,
            enqueuedAt: Date.now(),
            position: res.position,
          })
        );
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
  }, [text, attachments, sending, busy, sendInfo, session.id, queueOnlyNext]);

  // P2-7 失败对账（插队/撤回/修改共用）：失败后复核 /session-queue——
  // - 条目仍在 pending → 恢复排队视图（刷新队位，行同步更新，「立即发送/修改/撤回」
  //   按钮保留可重试）；
  // - 确认不在队（已被消费 / 他端撤回）→ onGone 终态（jump/retract 均为中性 gone
  //   收敛文案，评审裁决：不落 failed「可重试」，防重复注入）+ 行移除（确认不在队
  //   的行继续挂着按钮会指向必 404 的幽灵条目）；
  // - 复核自身网络失败 → 保守恢复排队视图（队位沿用旧值，3s 轮询随后自愈）——
  //   拿不到「真不在队」的证据就不落终态；行缺失时补回（含他从桌面排、本端无
  //   回执的条目），避免按钮丢失后排队条目在 UI 上失控
  const reconcileQueued = useCallback(
    async (itemId: number, prevPosition: number, prevContent: string, onGone: () => void) => {
      try {
        const items = await fetchQueue(session.id);
        const mine = items.find((i) => i.id === itemId);
        if (mine) {
          // 复核快照是点击时刻之后的服务端视图，比此前在途的 tick 更具新：落地即
          // 前移认知时刻，作废可能显示「不在队」的旧 tick（防假 gone）
          queueMutatedAtRef.current = Date.now();
          setReceipt({
            kind: "queued",
            itemId,
            position: mine.position,
            content: mine.content ?? prevContent,
          });
          setQueueItems((prev) =>
            upsertQueueItem(prev, {
              id: itemId,
              content: mine.content,
              enqueuedAt: mine.enqueuedAt,
              position: mine.position,
            })
          );
        } else {
          // 确认不在队同理前移：此后到货的旧 tick 快照仍显示在队 → 不得复活幽灵行
          queueMutatedAtRef.current = Date.now();
          setQueueItems((prev) => prev.filter((i) => i.id !== itemId));
          onGone();
        }
      } catch {
        // 保守恢复也是一次本地认知落地（「该条仍在队」）：同样作废此前在途旧快照
        queueMutatedAtRef.current = Date.now();
        setReceipt({ kind: "queued", itemId, position: prevPosition, content: prevContent });
        setQueueItems((prev) =>
          prev.some((i) => i.id === itemId)
            ? prev
            : upsertQueueItem(prev, {
                id: itemId,
                content: prevContent,
                enqueuedAt: 0,
                position: prevPosition,
              })
        );
      }
    },
    [session.id]
  );

  // D8 参数化（按条操作，不再读 receipt.itemId——桌面端/他端排的队同样可插队）。
  // busy 加闸单点保留：同一时刻至多一个排队操作在途
  const handleJump = useCallback(
    async (item: QueueItemView) => {
      if (busy) return;
      const { id: itemId, position, content } = item;
      setBusy(true);
      try {
        const j = await queueJump(session.id, itemId);
        if (j.status === "delivered") {
          // 本地认知翻转：该条已出队（时序防御见 queueMutatedAtRef 声明处）
          queueMutatedAtRef.current = Date.now();
          setReceipt({ kind: "delivered" });
          // 立即发送成功 = 条目已出队：行即时移除（3s 轮询权威列表随后兜底）
          setQueueItems((prev) => prev.filter((i) => i.id !== itemId));
        } else {
          // 非 delivered 回执先复核再定终态。queued（F1 后新语义：jump 守卫忙/
          // 条目已被消费 → 回 queued{itemId,position}，前端 reconcileQueued 对账
          // 兼容——条目仍在队恢复排队视图、确认不在队走中性 gone 收敛）与 failed
          // （注入失败，行已退出 pending）同走复核——不得落 failed「可重试」诱发
          // 重复注入（评审裁决）
          await reconcileQueued(itemId, position, content, () =>
            setReceipt({ kind: "gone", message: GONE_JUMP_MESSAGE })
          );
        }
      } catch {
        // 网络层 / 非 2xx（404 条目已不在队等）：同样复核，在队即恢复、不在队中性收敛
        await reconcileQueued(itemId, position, content, () =>
          setReceipt({ kind: "gone", message: GONE_JUMP_MESSAGE })
        );
      } finally {
        setBusy(false);
      }
    },
    [busy, session.id, reconcileQueued]
  );

  /** 撤回/修改共用的出队执行器（评审必须1 的复核收敛逻辑单点保留，两钮不复制）：
   *  调 queueRetract——{ok:true} = 服务端确认已撤（免复核直接收敛，撤回目的已达成）；
   *  忙时 failed / 404 / 网络错 → 复核对账（条目仍在队恢复排队视图可重试、确认不在队
   *  走中性 gone）。onConfirmed 仅在「确认出队」后被调：撤回用它清回执，
   *  修改用它把正文放回输入框 */
  const retractWithOutcome = useCallback(
    async (itemId: number, position: number, content: string, onConfirmed: () => void) => {
      setBusy(true);
      try {
        const r = await queueRetract(session.id, itemId);
        if ("ok" in r) {
          /* 服务端确认已撤（{ok:true}）：免复核，直接收敛（评审必须1——撤回目的已达成）；
             行同步移除（快路径不拉队列，列表即权威呈现）。本地认知翻转前移，作废
             可能仍显示在队的在途旧快照（M1：防幽灵行复活） */
          queueMutatedAtRef.current = Date.now();
          setQueueItems((prev) => prev.filter((i) => i.id !== itemId));
          onConfirmed();
        } else {
          /* 忙时 200 {status:"failed"}：条目未被撤、仍在队 → 复核对账。不得丢弃联合
             返回值直接复核——否则「忙时 + 复核也失败」双失败时条目实际在队、UI 却
             永久失控（评审必须1核心场景；reconcileQueued 复核失败保守恢复兜底） */
          await reconcileQueued(itemId, position, content, () =>
            setReceipt({ kind: "gone", message: GONE_RETRACT_MESSAGE })
          );
        }
      } catch (e) {
        if (e instanceof ApiError && e.status === 404) {
          /* 404 not_found（已送达 / 他端撤回）：条目已不在队 → 中性收敛（不作失败
             提示），行同步移除；本地认知翻转前移（同 M1 防幽灵行） */
          queueMutatedAtRef.current = Date.now();
          setQueueItems((prev) => prev.filter((i) => i.id !== itemId));
          setReceipt({ kind: "gone", message: GONE_RETRACT_MESSAGE });
        } else {
          // 网络错：同款复核——条目仍在队 → 恢复排队视图可重试；确认不在队 → 中性收敛
          await reconcileQueued(itemId, position, content, () =>
            setReceipt({ kind: "gone", message: GONE_RETRACT_MESSAGE })
          );
        }
      } finally {
        setBusy(false);
      }
    },
    [session.id, reconcileQueued]
  );

  /** 撤回（W4）：完全取消——确认出队后清回执 + 行移除，正文不保留（2026-09-20
   *  裁决：撤回=完全取消；拉回编辑走「修改」钮）。D8：按条操作，不限本端发送的
   *  条目 */
  const handleRetract = useCallback(
    async (item: QueueItemView) => {
      if (busy) return;
      await retractWithOutcome(item.id, item.position, item.content, () => setReceipt(null));
    },
    [busy, retractWithOutcome]
  );

  /** 修改（2026-09-20 裁决）：确认出队后把正文放回输入框继续编辑——与撤回共用
   *  同一后端出队动作，差别仅在是否恢复文本。D8：按条取行 content——放回前剥离
   *  [mobile …] 前缀（行 content 是服务端 compose 后的最终注入文本，不剥则重发
   *  二次叠加前缀）；附件标记行保留（重发仍引用同一落盘文件）。截断到
   *  MAX_SEND_CHARS 与输入框 maxLength 对齐；恢复后聚焦输入框（移动端直接可改）。
   *  D6：确认出队后置 queueOnlyNext——修改后的重发一律入队（防「文件说闲、
   *  TUI 实忙」窗口变相插队）；忙时失败/复核失败条目仍在队（onConfirmed 不触发）
   *  则不置标志，取消「修改」意图后的排队视图照旧 */
  const handleEdit = useCallback(
    async (item: QueueItemView) => {
      if (busy) return;
      await retractWithOutcome(item.id, item.position, item.content, () => {
        setText(stripMobilePrefix(item.content).slice(0, MAX_SEND_CHARS));
        setQueueOnlyNext(true);
        setReceipt(null);
        inputRef.current?.focus();
      });
    },
    [busy, retractWithOutcome]
  );

  // 附件上传（2026-09-20）：逐个上传 → chips 状态机（uploading → ready/failed）。
  // 404 no_cwd 一次即置 noCwd（会话无项目目录，+ 钮禁用——与 R5 禁用口径同源）；
  // 403 设备失效抛 ApiError(403, "设备已失效…") → failed chip（Board 侧另有 403
  // 全局判废通道，此处不重复处理）
  const addFiles = useCallback(
    async (files: File[]) => {
      for (const f of files) {
        const id = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
        const isImage = f.type.startsWith("image/");
        setAttachments((prev) => [
          ...prev,
          { id, name: f.name || (isImage ? "粘贴图片.png" : "file"), isImage, status: "uploading" },
        ]);
        // AbortController：上传中可取消（× 移除 = 中断上传 + 移除 chip）——
        // 大文件（视频）上传耗时长，不可取消会被迫干等（2026-09-20 用户实测）
        const ctrl = new AbortController();
        uploadCtrlsRef.current.set(id, ctrl);
        try {
          const res = await uploadAttachment(session.id, f, ctrl.signal);
          if (res === null) throw new ApiError(403, "设备已失效，请重新配对");
          setAttachments((prev) =>
            prev.map((a) => (a.id === id ? { ...a, status: "ready", path: res.path } : a))
          );
        } catch (e) {
          if (ctrl.signal.aborted) {
            // 用户取消：chip 已随 removeAttachment 移除，静默收尾
            setAttachments((prev) => prev.filter((a) => a.id !== id));
            continue;
          }
          const isNoCwd = e instanceof ApiError && e.status === 404 && e.message === "no_cwd";
          if (isNoCwd) setNoCwd(true);
          const reason =
            e instanceof ApiError && isNoCwd
              ? "该会话没有项目目录信息"
              : String(e instanceof ApiError ? e.message : e);
          setAttachments((prev) =>
            prev.map((a) => (a.id === id ? { ...a, status: "failed", error: reason } : a))
          );
        } finally {
          uploadCtrlsRef.current.delete(id);
        }
      }
    },
    [session.id]
  );

  const removeAttachment = useCallback((id: string) => {
    // 上传中移除 = 取消：中断在途 fetch，防止白传到底（2026-09-20 用户反馈）
    uploadCtrlsRef.current.get(id)?.abort();
    uploadCtrlsRef.current.delete(id);
    setAttachments((prev) => prev.filter((a) => a.id !== id));
  }, []);

  /** 粘贴图片（2026-09-20）：clipboard 里的图片文件走同上传链路（桌面浏览器粘贴
   *  最顺；移动端以 + 钮文件选择为主）。非图片粘贴放行默认文本行为 */
  const handlePaste = useCallback(
    (e: ReactClipboardEvent<HTMLTextAreaElement>) => {
      const images = Array.from(e.clipboardData?.files ?? []).filter((f) =>
        f.type.startsWith("image/")
      );
      if (images.length === 0) return;
      e.preventDefault();
      void addFiles(images);
    },
    [addFiles]
  );

  // 拉取未就绪 / 失败 / 403：不渲染（详情页正文照常）
  if (!infoReady || sendInfo === null) return null;

  const canSend =
    injectable &&
    !sending &&
    !busy &&
    text.trim().length > 0 &&
    !attachments.some((a) => a.status === "uploading");

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
      {/* 单回执槽（T4 复评 M3 语义对齐）：保留**最近一次队列操作**的终态——本端
          发送，或对任意行（含他端排队条目）的立即发送/修改/撤回：
          delivered/submitted/failed/gone。queued 态不再渲染 chip（由下方多列队
          列表取代），回执状态本身保留——乐观行即时反馈、失败对账与 3s 轮询仍以
          其驱动 */}
      {receipt !== null && receipt.kind !== "queued" && (
        <div className="mb-2 flex flex-wrap items-center gap-2">
          {receipt.kind === "delivered" && (
            <span
              data-testid="send-receipt-delivered"
              className="rounded-full bg-emerald-500/10 px-2 py-0.5 text-xs text-emerald-700 dark:bg-emerald-400/10 dark:text-emerald-400"
            >
              已送达终端
            </span>
          )}
          {receipt.kind === "submitted" && (
            // D7/T3 中性回执（非确认成功亦非失败）：已投递未确认——agent 空闲后
            // 处理 TUI 内部队列里的消息；配色与 gone 同族（中性 slate），不带
            // 「（可重试）」——该语义仅属于 failed 态，出现在此会诱导双发
            <span
              data-testid="send-receipt-submitted"
              className="rounded-full bg-slate-200/70 px-2 py-0.5 text-xs text-slate-600 dark:bg-slate-700/60 dark:text-slate-300"
            >
              已投递至终端输入，agent 空闲后处理（未确认落盘）
            </span>
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
      {/* D8 多列队列表（验收问题 #6）：完整 /session-queue 渲染——逐条预览 + 位次 +
          立即发送/修改/撤回三钮（按条 id，不限本端发送的条目）；「排队中 第 N 位」
          chip 已由本列表取代。位次沿用 position=0 容忍口径（并发消费窗口值 →
          「排队中」不带数字） */}
      {queueItems.length > 0 && (
        <div className="mb-2" data-testid="queue-list">
          <ul className="space-y-1">
            {queueItems.map((item) => (
              <li
                key={item.id}
                data-testid={`queue-row-${item.id}`}
                className="flex flex-wrap items-center gap-1.5 rounded-lg bg-amber-500/10 px-2 py-1 dark:bg-amber-400/10"
              >
                <span
                  data-testid={`queue-position-${item.id}`}
                  className="shrink-0 text-xs text-amber-700 dark:text-amber-300"
                >
                  {item.position >= 1 ? `第${item.position}位` : "排队中"}
                </span>
                <span
                  className="min-w-0 flex-1 truncate text-xs text-slate-700 dark:text-slate-300"
                  title={item.content}
                >
                  {queueItemPreview(item.content)}
                </span>
                <button
                  type="button"
                  data-testid="queue-jump"
                  disabled={busy || sending}
                  onClick={() => {
                    void handleJump(item);
                  }}
                  className="rounded-full bg-amber-500/20 px-2 py-0.5 text-xs text-amber-700 disabled:opacity-40 dark:bg-amber-400/20 dark:text-amber-300"
                >
                  立即发送
                </button>
                <button
                  type="button"
                  data-testid="queue-edit"
                  disabled={busy || sending}
                  onClick={() => {
                    void handleEdit(item);
                  }}
                  className="rounded-full bg-amber-500/20 px-2 py-0.5 text-xs text-amber-700 disabled:opacity-40 dark:bg-amber-400/20 dark:text-amber-300"
                >
                  修改
                </button>
                <button
                  type="button"
                  data-testid="queue-retract"
                  disabled={busy || sending}
                  onClick={() => {
                    void handleRetract(item);
                  }}
                  className="rounded-full bg-slate-200 px-2 py-0.5 text-xs text-slate-600 disabled:opacity-40 dark:bg-slate-800 dark:text-slate-300"
                >
                  撤回
                </button>
              </li>
            ))}
          </ul>
          <p
            data-testid="queue-hint"
            className="mt-1 text-[11px] text-slate-400 dark:text-slate-500"
          >
            空闲时将按序自动发送
          </p>
        </div>
      )}
      {/* 待发附件 chips（2026-09-20）：上传中/就绪/失败三态，可单个移除 */}
      {attachments.length > 0 && (
        <div className="mb-1 flex flex-wrap items-center gap-1.5" data-testid="attachment-chips">
          {attachments.map((a) => (
            <span
              key={a.id}
              data-testid={`attach-chip-${a.id}`}
              className={`flex items-center gap-1 rounded-full px-2 py-0.5 text-xs ${
                a.status === "failed"
                  ? "bg-rose-500/10 text-rose-700 dark:bg-rose-400/10 dark:text-rose-400"
                  : "bg-slate-200 text-slate-600 dark:bg-slate-800 dark:text-slate-300"
              }`}
            >
              {a.status === "uploading"
                ? `上传中：${a.name}`
                : a.status === "failed"
                  ? `失败：${a.name}（${a.error}）`
                  : a.name}
              <button
                type="button"
                data-testid={`attach-remove-${a.id}`}
                aria-label={`移除附件 ${a.name}`}
                onClick={() => removeAttachment(a.id)}
                className="text-slate-400 hover:text-slate-600 dark:text-slate-500"
              >
                ×
              </button>
            </span>
          ))}
        </div>
      )}
      <div className="flex items-end gap-2">
        {/* 附件入口（2026-09-20）：+ 选择文件；「?」知情披露（存储到用户项目目录） */}
        <span className="flex shrink-0 items-center gap-0.5">
          <input
            ref={fileInputRef}
            type="file"
            multiple
            className="hidden"
            data-testid="attach-file-input"
            onChange={(e) => {
              const fs = Array.from(e.target.files ?? []);
              e.target.value = "";
              if (fs.length > 0) void addFiles(fs);
            }}
          />
          <button
            type="button"
            data-testid="attach-add"
            aria-label="添加附件"
            aria-expanded={attachHintOpen}
            disabled={!injectable || noCwd}
            title={
              noCwd
                ? "该会话没有项目目录信息，无法上传附件"
                : "添加附件（保存到项目目录 .mam-attachments/）"
            }
            onClick={() => fileInputRef.current?.click()}
            className="shrink-0 rounded-full p-1 text-slate-500 hover:bg-slate-200 disabled:opacity-40 dark:text-slate-400 dark:hover:bg-slate-800"
          >
            <Plus size={16} />
          </button>
          <button
            type="button"
            data-testid="attach-help"
            aria-label="附件存储说明"
            aria-expanded={attachHintOpen}
            onClick={() => setAttachHintOpen((v) => !v)}
            className="shrink-0 rounded-full p-0.5 text-[10px] leading-none text-slate-400 hover:bg-slate-200 dark:text-slate-500 dark:hover:bg-slate-800"
          >
            <CircleHelp size={12} />
          </button>
        </span>
        <textarea
          ref={inputRef}
          data-testid="composer-input"
          aria-label="消息输入"
          value={text}
          maxLength={MAX_SEND_CHARS}
          onChange={(e) => setText(e.target.value.slice(0, MAX_SEND_CHARS))}
          onPaste={handlePaste}
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
      {/* 知情披露（2026-09-20 用户要求）：「?」点开才显示——附件落在用户项目目录，
          已本地 git 排除不会提交；项目收尾可整目录清理。看过即收、不平铺常驻 */}
      {attachHintOpen && (
        <p
          data-testid="attach-hint"
          className="mt-1 rounded-lg bg-slate-100 px-2 py-1.5 text-[11px] leading-4 text-slate-500 dark:bg-slate-900 dark:text-slate-400"
        >
          附件将保存到用户项目目录 .mam-attachments/&lt;会话&gt;/（已在本地 git
          排除，不会提交）；项目收尾时可整目录清理。
        </p>
      )}
    </div>
  );
}
