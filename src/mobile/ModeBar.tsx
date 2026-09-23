import { useCallback, useEffect, useState } from "react";
import { toneTokens } from "./InteractiveCard";
import {
  ApiError,
  fetchSessionMode,
  fetchSessionModeMenu,
  sessionModeMenuOpen,
  sessionModeMenuPick,
  sessionModeSwitch,
  type MamMode,
  type ModeGroupId,
  type ModeGroupView,
  type ModeMenuOption,
  type SessionModeView,
} from "./api";

/** 模式栏（批次丙 T6；批次丁 T4 扩二维与回读全开）。
 *
 * 数据源 GET /session-mode。**丁T4 起后端下发结构表**（§2.6 规格表）：
 * - 二维家（codex/kimi）→ `structure="twoAxis"`，`groups` = [模式组, 权限组]，
 *   两组各自显示当前档 + 各自的按钮；
 * - 单轴家（claude/opencode）→ `structure="singleAxis"`，`groups` = [模式轴]，
 *   渲染「切换」钮 + 当前模式回显；
 * - 旧后端（无 `structure`/`groups`）→ **回落**到「单轴渲染 + 顶层 current」，
 *   调用 POST 时不带 `group`（后端按 target 归组）。
 *
 * **降级语义（T6 红线 4 + 裁5：不假装成功）**：
 * - `switchKind === "unsupported"` → 不渲染（该工具无实测切换机制）；
 * - 某组 `current === null` → 该组显示「模式未知」+「请人工核对终端」；
 * - 切档回执 `verified === false` → 显示后端下发的 `hint`（含「人工核对」语义），
 *   **不声称已切到目标档**。
 *
 * **裁7（codex 退役旧档不作可选）**：`groups[].tiers[]` 里 `selectable=false` 的档
 * 不渲染为可点按钮（它的 `reason` 会作为灰字提示显示，让用户知道为什么点不了）；
 * `groups[].legacy[]`（untrusted/on-failure）只渲染为一行说明文本——**不是按钮**。
 */
export default function ModeBar({ session }: { session: { id: string } }) {
  const [view, setView] = useState<SessionModeView | null>(null);
  const [ready, setReady] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** 切档回执提示（成功后展示；verified=false 时是人工核对提示） */
  const [receipt, setReceipt] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    fetchSessionMode(session.id)
      .then((v) => {
        if (alive) setView(v);
      })
      .catch(() => {
        /* 拉取失败静默自隐（approve/question 同惯例） */
      })
      .finally(() => {
        if (alive) setReady(true);
      });
    return () => {
      alive = false;
    };
  }, [session.id]);

  // 后端是否下发了结构表（旧后端没有 → 前端合成的单轴视图，POST 不带 `group`）。
  // 抽成**布尔**再进依赖数组：直接依赖 `view?.groups` 会让 useCallback 的依赖
  // 每次渲染都变（数组字面量），等于没 memo（eslint react-hooks 会点名）
  const hasServerGroups = view?.groups !== undefined;

  const handleSwitch = useCallback(
    async (target: MamMode, group?: ModeGroupId) => {
      setBusy(true);
      setError(null);
      setReceipt(null);
      try {
        // 旧后端（无 groups 字段）回落出来的那组是前端**合成的**——它的 id 不是
        // 后端给的，故不带 `group` 发（后端按 target 自行归组，与旧调用逐字一致）
        const g = hasServerGroups ? group : undefined;
        const res = await sessionModeSwitch(session.id, target, g);
        if (res.status === "key_sent") {
          if (res.verified) {
            // 回读命中：回执 + 重拉一次 GET 刷新显示（**不拿回执字段去改本地状态**：
            // 回执只描述该组那一刻的观测，直接采信会把「屏读快照」当成结构表；
            // 重拉失败则保留原视图——如实，不是把旧值刷成新值）
            setReceipt("已切换");
            try {
              setView(await fetchSessionMode(session.id));
            } catch {
              /* 回读失败不清回执（切换本身已投递且已核实） */
            }
          } else {
            // 红线 4：不假装成功——原样透出后端 hint（含「预期/实际」或「请人工核对」）
            setReceipt(res.hint ?? "已发送切换，请人工核对终端模式");
          }
        } else {
          setError(res.error);
        }
      } catch (e) {
        if (e instanceof ApiError) {
          if (e.status === 404 && e.data?.error === "no_session") {
            setError("会话已结束，请返回看板刷新");
          } else if (e.status === 409 && e.data?.error === "no_mechanism") {
            setError("该工具的模式切换未实测");
          } else if (e.status === 409 && e.data?.error === "blocked_by_dialog") {
            // 丁T3 §2.7 对话框在场红线（裁8/9，问题 5）：控制类注入被拒——后端
            // 屏读见编号选项对话框，切换键/斜杠命令会落进对话框（实机连点 17 次
            // 全变「选第一项」）。文案用后端下发的 reason（单一来源），缺省给同义兜底
            setError(
              typeof e.data?.reason === "string" ? e.data.reason : "终端有待决对话框，请先处理"
            );
          } else if (e.status === 409 && e.data?.error === "blocked_by_question") {
            // E3② 待决拦截（硬）：kimi 问答待决时切档注入（含回车）会误选答案——
            // 后端整条拒绝；文案用后端 reason（单一来源）
            setError(
              typeof e.data?.reason === "string"
                ? e.data.reason
                : "终端有待答的问题，请先在问答卡作答"
            );
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
    [session.id, hasServerGroups]
  );

  // 未就绪 / 拉取失败 / 无实测机制：不渲染
  if (!ready || view === null || view.switchKind === "unsupported") return null;

  // T10：本组件是**状态条**（非「等待用户输入」交互卡，任务书 §2.2 的容器契约
  // 针对 ApproveCard/QuestionCard 两类交互卡），故保留自身的横向布局；但**取色
  // 走统一 token**（InteractiveCard 的 mode 档）——四套界面同一套设计语言
  const t = toneTokens("mode");
  const groups = modeGroups(view);
  // E3④ 待决置灰：终端问答待决时切档注入（含回车）会被问答框误消费
  // （codex 交默认答案 / kimi 误选推进待决态）→ 全部按钮禁用 + 原因文案。
  // undefined（旧后端）= 未知，不置灰（与后端「无法判定放行」同一取向）
  const questionPending = view.questionPending === true;
  return (
    <div
      data-testid="mode-bar"
      data-mode={view.current ?? "unknown"}
      data-structure={view.structure ?? "legacy"}
      data-question-pending={questionPending ? "true" : "false"}
      data-tone="mode"
      className={`flex flex-wrap items-center gap-x-3 gap-y-1 px-2 py-1 ${t.box}`}
    >
      {groups.map((g) => (
        <ModeGroupRow
          key={g.id}
          group={g}
          showGroupLabel={groups.length > 1}
          busy={busy || questionPending}
          sessionId={session.id}
          onSwitch={(target) => handleSwitch(target, g.id)}
          onPicked={(hint, ok) => {
            // picker 走完：ok=true 时刷新视图（后端可能已改档，重拉 GET 拿权威结构），
            // 否则只显示后端原样文案——**不假装成功**（与 handleSwitch 同红线）
            if (ok) {
              setError(null);
              setReceipt(hint);
              void fetchSessionMode(session.id)
                .then(setView)
                .catch(() => {
                  /* 重拉失败保留原视图——如实，不是把旧值刷成新值 */
                });
            } else {
              setReceipt(null);
              setError(hint);
            }
          }}
        />
      ))}
      {questionPending && (
        <span
          data-testid="mode-question-pending-hint"
          className="text-[11px] text-amber-700 dark:text-amber-400"
        >
          终端有待答的问题——切权限/模式的命令会误选答案，请先在问答卡作答
        </span>
      )}
      {error !== null && (
        <span data-testid="mode-error" className="text-[11px] text-rose-600 dark:text-rose-400">
          {error}
        </span>
      )}
      {receipt !== null && (
        <span
          data-testid="mode-receipt"
          className="text-[11px] text-emerald-600 dark:text-emerald-400"
        >
          {receipt}
        </span>
      )}
    </div>
  );
}

/** 视图 → 组清单。
 *
 * 丁T4 的结构表来自后端；旧后端（无 `groups`）在此**回落到单轴视图**（用顶层
 * current/readback + switchKind 合成一组）——这样渲染分支只有一套，不会出现
 * 「新老两条渲染路径」的分叉（分叉正是口径漂移的温床）。
 *
 * # 前向兼容的**如实边界**（T4 复评 M1：这不是「零损失」，是**降级**）
 *
 * 回落路径用 `step = switchKind === "shiftTab"` 决定渲染哪条分支，且 `tiers: []`
 * （旧后端不下发档位表，前端无从知道该工具有哪些档、屏显标签叫什么）。后果：
 * - `shiftTab` 的旧后端（claude / opencode，**以及 T4 之前的 kimi**）→ 渲染
 *   「切换模式」钮 + 顶层 current 回显，与旧前端行为一致（**无损失**）；
 * - `slashCommand` 的旧后端（codex）→ 旧前端渲染的是 `plan`/`bypass` **两个逐档按钮**
 *   （`/plan` 与 `/permissions` 两条有命令证据的路），而 `tiers: []` 让新前端**一个
 *   逐档按钮都渲染不出来** —— 只剩回显。**这是功能降级，不是等价兼容。**
 *
 * 为什么接受这个降级而不做「旧后端硬编码一份 codex 档位表」：那份表会立刻成为第二份
 * 真源（§2.6 的规格表 + 后端结构表已经是两份，再加前端硬编码就是三份），而后端升级
 * 到 T4 后它就变成**永不执行的死代码**——本仓既往的口径漂移大多源于这种「过渡期硬
 * 编码」。过渡期的正确做法是**如实告知**：旧后端 + codex 时 ModeBar 只回显不给切换
 * 入口，用户在终端操作或升级后端即可。 */
function modeGroups(view: SessionModeView): ModeGroupView[] {
  if (view.groups !== undefined && view.groups.length > 0) return view.groups;
  // 旧后端回落：单轴 + 顶层 current。tiers 空（旧后端不下发档位表）——
  // `slashCommand` 的旧后端（codex）因此失去逐档按钮（见上方如实边界）。
  return [
    {
      id: "mode",
      label: "模式",
      step: view.switchKind === "shiftTab",
      readback: view.readback,
      current: view.current,
      currentLabel: view.currentLabel,
      tiers: [],
      legacy: [],
    },
  ];
}

/** 单组渲染：组标题（仅二维时显示）+ 当前档 + 切换入口。
 *  E3④：当前档按钮**高亮**（data-current + 反色样式）；问答待决时全组禁用。
 *  2026-09-23 codex 模式切换改造新增三个面：
 *  - `layout === "toggle"`（codex 模式组）→ 单钮「计划 ⇄ 操作」（点击 = 向终端发一次
 *    shift+tab，目标档按当前档翻转；**当前档未知时禁用**——盲按会 50% 误切）；
 *  - 「完全信任」二次确认（用户裁决）：点完全信任先出确认条，确认后才发——codex 会
 *    连发两次按键（4→1）并代按终端的风险确认框；
 *  - 权限组（无屏读源）current 有值时标注「（上次切换）」——它是 MAM 的记忆，不是
 *    实时屏读（终端手改会失真，如实声明口径）。 */
function ModeGroupRow({
  group,
  showGroupLabel,
  busy,
  sessionId,
  onSwitch,
  onPicked,
}: {
  group: ModeGroupView;
  showGroupLabel: boolean;
  /** E3④：busy 含问答待决置灰（父层合并——待决时全组按钮禁用） */
  busy: boolean;
  /** 单选面板（picker）要用会话 id 调 /session-mode/menu */
  sessionId: string;
  onSwitch: (target: MamMode) => void;
  /** picker 走完（done/failed）→ 父层刷新视图与回执 */
  onPicked: (hint: string, ok: boolean) => void;
}) {
  const currentText = group.currentLabel ?? "模式未知";
  const unknown = group.current === null;
  // 完全信任二次确认的待确认态（只在含 bypass 档的权限组用得上）
  const [bypassArmed, setBypassArmed] = useState(false);
  const hasBypass = group.tiers.some((t) => t.mode === "bypass" && t.selectable);
  return (
    <span data-testid={`mode-group-${group.id}`} className="flex flex-wrap items-center gap-2">
      {showGroupLabel && (
        <span className="text-[11px] text-slate-400 dark:text-slate-500">{group.label}</span>
      )}
      {/* 单组（无组标题）时给当前档一个「当前」前缀；二维两行组已有组标题
          （模式/权限），再叠「模式」二字会读成「模式 模式 …」（用户 2026-09-23） */}
      {!showGroupLabel && <span className="text-xs text-slate-500 dark:text-slate-400">当前</span>}
      <span
        data-testid={`mode-current-${group.id}`}
        className={`text-xs font-semibold ${
          unknown ? "text-amber-700 dark:text-amber-400" : "text-slate-800 dark:text-slate-200"
        }`}
      >
        {currentText}
      </span>
      {!unknown && group.id === "permission" && group.readback === false && (
        <span
          data-testid={`mode-current-source-${group.id}`}
          className="text-[10px] text-slate-400 dark:text-slate-500"
        >
          （上次切换）
        </span>
      )}
      {unknown && (
        <span
          data-testid={`mode-unknown-hint-${group.id}`}
          className="text-[11px] text-amber-700 dark:text-amber-400"
        >
          请人工核对终端当前模式
        </span>
      )}
      {group.layout === "picker" ? (
        // **单选面板**（2026-09-23 用户方案）：单钮「切换权限」→ 后端读回终端菜单的
        // 选项表 → 用户点选哪项就敲哪个数字键。前端**不硬编码「哪档对应哪个数字」**
        // （档位编号随 Guardian 配置前移，硬编码会错位）。
        <PermissionPicker sessionId={sessionId} disabled={busy} onDone={onPicked} />
      ) : group.layout === "toggle" ? (        // 单钮 toggle（codex 模式组）：点击向终端发一次 shift+tab，终端在
        // 计划/操作间循环；目标档按当前档翻转（current 未知 → 禁用，防盲按误切）
        (() => {
          const toggleTarget: MamMode = group.current === "plan" ? "default" : "plan";
          return (
            <button
              type="button"
              data-testid={`mode-switch-${group.id}-toggle`}
              data-current={group.current ?? "unknown"}
              disabled={busy || unknown}
              title={
                unknown
                  ? "请先人工核对终端当前模式（当前档未知时盲按会误切）"
                  : "向终端发送 shift+tab，在计划/操作间切换"
              }
              onClick={() => onSwitch(toggleTarget)}
              className="rounded-full bg-blue-600 px-2.5 py-0.5 text-[11px] font-semibold text-white disabled:opacity-40 dark:bg-blue-500"
            >
              计划 ⇄ 操作
            </button>
          );
        })()
      ) : group.step ? (
        // 步进轴（claude/opencode）：单钮「切换模式」——shift+tab 一次一档，
        // 目标档由环序决定（后端按实测环序回读核对，前端不假装直达）
        <button
          type="button"
          data-testid={`mode-switch-next-${group.id}`}
          disabled={busy}
          onClick={() => onSwitch("default")}
          className="rounded-full bg-slate-500/15 px-2 py-0.5 text-[11px] text-slate-700 disabled:opacity-40 dark:bg-slate-400/15 dark:text-slate-300"
        >
          切换模式
        </button>
      ) : (
        // 直达轴（模式组/权限组）：逐档按钮。**只渲染 selectable 的档**（裁7 的
        // 退役档根本不进 tiers）。
        // **当前档高亮**（E3④）：`group.current === t.mode` 的按钮反色 + data-current。
        // 「完全信任」特例（2026-09-23 用户裁决）：先武装二次确认条，确认后才发。
        <span className="flex flex-wrap items-center gap-1">
          {group.tiers.map((t) => {
            const isCurrent = group.current !== null && group.current === t.mode;
            return t.selectable ? (
              <button
                key={t.mode}
                type="button"
                data-testid={`mode-switch-${group.id}-${t.mode}`}
                data-current={isCurrent ? "true" : "false"}
                disabled={busy}
                onClick={() => {
                  if (t.mode === "bypass") {
                    setBypassArmed(true); // 不直接发——等「确认启用」
                  } else {
                    onSwitch(t.mode);
                  }
                }}
                className={`rounded-full px-2 py-0.5 text-[11px] disabled:opacity-40 ${
                  isCurrent
                    ? "bg-slate-700 font-semibold text-white dark:bg-slate-200 dark:text-slate-900"
                    : "bg-slate-500/15 text-slate-700 dark:bg-slate-400/15 dark:text-slate-300"
                }`}
              >
                {t.label}
              </button>
            ) : (
              // 不可选档：不渲染按钮，只给一句灰字（reason 来自后端，单一来源）
              <span
                key={t.mode}
                data-testid={`mode-tier-disabled-${group.id}-${t.mode}`}
                title={t.reason ?? undefined}
                className="rounded-full border border-dashed border-slate-400/40 px-2 py-0.5 text-[11px] text-slate-400 dark:text-slate-500"
              >
                {t.label}（不可用）
              </span>
            );
          })}
        </span>
      )}
      {/* 裁7：退役旧档**只作说明**，不渲染为可点按钮 */}
      {group.legacy !== undefined && group.legacy.length > 0 && (
        <span
          data-testid={`mode-legacy-${group.id}`}
          className="text-[11px] text-slate-400 dark:text-slate-500"
        >
          已退役：{group.legacy.map((l) => l.label).join(" / ")}
        </span>
      )}
      {/* 完全信任二次确认条（2026-09-23 用户裁决的前端防线；未确认不发任何请求） */}
      {hasBypass && bypassArmed && (
        <span
          data-testid="mode-bypass-confirm"
          className="flex w-full flex-wrap items-center gap-2 rounded-lg bg-rose-500/10 px-2 py-1 text-[11px] text-rose-700 dark:text-rose-300"
        >
          <span>将连发两次按键（4→1）启用完全信任：终端会弹出风险确认框，由 MAM 代按确认。</span>
          <button
            type="button"
            data-testid="mode-bypass-confirm-yes"
            disabled={busy}
            onClick={() => {
              setBypassArmed(false);
              onSwitch("bypass");
            }}
            className="rounded-full bg-rose-600 px-2 py-0.5 font-semibold text-white disabled:opacity-40 dark:bg-rose-500"
          >
            确认启用
          </button>
          <button
            type="button"
            data-testid="mode-bypass-confirm-no"
            disabled={busy}
            onClick={() => setBypassArmed(false)}
            className="rounded-full bg-slate-500/15 px-2 py-0.5 text-slate-700 disabled:opacity-40 dark:bg-slate-400/15 dark:text-slate-300"
          >
            取消
          </button>
        </span>
      )}
    </span>
  );
}

/** **终端菜单单选面板**（codex 权限组，2026-09-23 用户方案）。
 *
 * 交互：单钮「切换权限」→ 点开后后端注入 `/permissions` + 回车并**读回终端菜单的
 * 选项表**（编号 = 屏上实读值、文本 = 屏上原文）→ 这里渲染成一列编号按钮 →
 * 用户点哪项，MAM 就敲哪个数字键。
 *
 * # 为什么这样比「后端自己敲」好（用户实机走查的结论）
 *
 * 旧路径是后端按目标档**猜**屏上编号（前端文案还硬编码「4→1」）。而档位编号会随
 * Guardian 配置前移（`Approve for me` 缺席时 `Full Access` 从 4 变 3）——猜错的
 * 后果是切到**别的档**（用户点只读、实际启用完全信任）。本面板把这一步交给用户：
 * 编号来自屏幕实读，MAM 只是投递，**不猜**。
 *
 * # 两阶段
 *
 * 切 Full Access 时终端会弹风险确认框 → 后端返回 `status:"confirm"` → 面板切为
 * 确认框的选项（同样读自终端原文），由用户再点一次。**MAM 不代按**。
 *
 * # 面板形态
 *
 * 内联展开（不引入遮罩/portal——沿用移动端既有惯例），配色与审批卡的「终端对话框
 * 选项」同款（那是现成的同形先例：编号徽标 + 纵向按钮 + 「点按即代你按对应数字键」）。
 */
function PermissionPicker({
  sessionId,
  disabled,
  onDone,
}: {
  sessionId: string;
  disabled: boolean;
  onDone: (hint: string, ok: boolean) => void;
}) {
  /** 面板态：null = 收起；menu = 菜单选项；confirm = 二阶段确认框选项 */
  const [panel, setPanel] = useState<null | "menu" | "confirm">(null);
  const [options, setOptions] = useState<ModeMenuOption[]>([]);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  const runOpen = useCallback(async () => {
    setBusy(true);
    setNote(null);
    try {
      const r = await sessionModeMenuOpen(sessionId);
      if (r.status === "menu") {
        setOptions(r.options);
        setPanel("menu");
      } else if (r.status === "confirm") {
        // 开菜单时确认框已在屏（上次残留）——直接进二阶段
        setOptions(r.options);
        setPanel("confirm");
        setNote(r.hint ?? null);
      } else if (r.status === "failed") {
        setNote(r.error);
        setPanel(null);
      } else {
        setNote("终端没有可读的权限菜单——请人工核对终端");
        setPanel(null);
      }
    } catch (e) {
      setNote(menuErrorText(e));
      setPanel(null);
    } finally {
      setBusy(false);
    }
  }, [sessionId]);

  const runReload = useCallback(async () => {
    setBusy(true);
    setNote(null);
    try {
      const r = await fetchSessionModeMenu(sessionId);
      if (r.status === "menu") {
        setOptions(r.options);
        setPanel("menu");
      } else if (r.status === "confirm") {
        setOptions(r.options);
        setPanel("confirm");
      } else if (r.status === "none") {
        setNote("终端屏上现在没有权限菜单或确认框（可能已关闭）——可重新打开");
      } else if (r.status === "failed") {
        setNote(r.error);
      }
    } catch (e) {
      setNote(menuErrorText(e));
    } finally {
      setBusy(false);
    }
  }, [sessionId]);

  const runPick = useCallback(
    async (number: number) => {
      setBusy(true);
      setNote(null);
      try {
        const r = await sessionModeMenuPick(sessionId, number);
        if (r.status === "confirm") {
          setOptions(r.options);
          setPanel("confirm");
          setNote(r.hint ?? "终端弹出风险确认框——请再点一次确认项");
        } else if (r.status === "done") {
          setPanel(null);
          setOptions([]);
          onDone(r.hint ?? (r.verified ? "已切换" : "已投递，请人工核对终端"), r.verified);
        } else if (r.status === "failed") {
          // 失败**保留面板**（用户可重读或重选——不逼他重新开菜单）
          setNote(r.error);
        } else {
          setNote("终端屏上没有菜单或确认框——请点「重新读取」");
        }
      } catch (e) {
        setNote(menuErrorText(e));
      } finally {
        setBusy(false);
      }
    },
    [sessionId, onDone]
  );

  const t = toneTokens("question");
  const isConfirm = panel === "confirm";
  return (
    <>
      <button
        type="button"
        data-testid="mode-picker-permission-open"
        disabled={disabled || busy}
        title="向终端发送 /permissions 打开权限菜单，然后由你点选档位"
        onClick={() => void runOpen()}
        className="rounded-full bg-blue-600 px-2.5 py-0.5 text-[11px] font-semibold text-white disabled:opacity-40 dark:bg-blue-500"
      >
        {busy && panel === null ? "读取终端菜单…" : "切换权限"}
      </button>
      {panel !== null && (
        <div
          data-testid="mode-menu-panel"
          data-panel={panel}
          className={`mt-1 w-full rounded-lg px-2 py-1.5 ${t.box}`}
        >
          <div className="mb-1 flex items-center justify-between gap-2">
            <span className="text-[11px] font-semibold text-sky-700 dark:text-sky-300">
              {isConfirm ? "终端二次确认" : "终端权限菜单"}
            </span>
            <span className="flex gap-1">
              <button
                type="button"
                data-testid="mode-menu-reload"
                disabled={busy}
                onClick={() => void runReload()}
                className="rounded-full bg-slate-500/15 px-2 py-0.5 text-[11px] text-slate-700 disabled:opacity-40 dark:bg-slate-400/15 dark:text-slate-300"
              >
                重新读取
              </button>
              <button
                type="button"
                data-testid="mode-menu-close"
                disabled={busy}
                onClick={() => {
                  setPanel(null);
                  setOptions([]);
                  setNote(null);
                }}
                className="rounded-full bg-slate-500/15 px-2 py-0.5 text-[11px] text-slate-700 disabled:opacity-40 dark:bg-slate-400/15 dark:text-slate-300"
              >
                关闭
              </button>
            </span>
          </div>
          <p className="mb-1 text-[11px] text-sky-700/80 dark:text-sky-400/80">
            {isConfirm
              ? "以下选项读自终端的风险确认框，点按即代你按对应数字键"
              : "以下选项读自终端，点按即代你按对应数字键（编号 = 屏幕上那个数字）"}
          </p>
          <div className="space-y-1">
            {options.length === 0 && (
              <p className="text-[11px] text-amber-700 dark:text-amber-400">
                未读到选项——请点「重新读取」再选
              </p>
            )}
            {options.map((o) => (
              <button
                key={o.number}
                type="button"
                data-testid={`mode-menu-option-${o.number}`}
                data-highlighted={o.highlighted ? "true" : "false"}
                disabled={busy}
                onClick={() => void runPick(o.number)}
                className={`flex w-full items-start gap-2 rounded-lg px-2 py-1.5 text-left text-xs disabled:opacity-40 ${t.action}`}
              >
                <span
                  className={`shrink-0 rounded px-1.5 py-0.5 font-mono text-[10px] font-semibold ${t.badge}`}
                >
                  {o.number}
                </span>
                <span className="min-w-0 flex-1 break-words">{o.label}</span>
              </button>
            ))}
          </div>
          {note !== null && (
            <p
              data-testid="mode-menu-note"
              className="mt-1 text-[11px] text-amber-700 dark:text-amber-400"
            >
              {note}
            </p>
          )}
        </div>
      )}
    </>
  );
}

/** 面板的 ApiError → 中文文案（错误码分诊与 `handleSwitch` 同口径：不把「网络挂了」
 *  和「终端有对话框」说成一句话）。 */
function menuErrorText(e: unknown): string {
  if (e instanceof ApiError) {
    if (e.status === 404 && e.data?.error === "no_session") {
      return "会话已结束，请返回看板刷新";
    }
    if (e.status === 409 && e.data?.error === "no_mechanism") {
      return typeof e.data?.reason === "string" ? e.data.reason : "该工具不支持终端菜单选择";
    }
    if (e.status === 409 && e.data?.error === "blocked_by_dialog") {
      return typeof e.data?.reason === "string" ? e.data.reason : "终端有待决对话框，请先处理";
    }
    return e.message;
  }
  return String(e);
}
