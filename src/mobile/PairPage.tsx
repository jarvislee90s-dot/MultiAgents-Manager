import { useCallback, useEffect, useRef, useState } from "react";
import { ApiError, pairWithPin } from "./api";

interface PairPageProps {
  /** 配对成功后回调：App 重新探测并切换到已配对态 */
  onPaired: () => void;
  /** M5 P3-b：App 级配对探测进行中（paired===null）——渲染连接指示器而非密码
   *  表单，区分「探测中」与「未配对」（实测走隧道探测有一二十秒延迟，探测期
   *  出密码页像「时滞掉线」）；探测完成（true/false）后按状态正常渲染 */
  probing?: boolean;
}

type PairStatus = "idle" | "pairing" | "ok";

// M5 A7（线稿 v5 移动端配对页）：密码单入口——旧「请求接入 / 4 位确认码」UI 与
// 扫码 token 流（#token=）随 M4 审批制一并下线，扫码参数改为 #pin=<密码>。
// 错误文案按 /pair/pin 响应分診（401 invalid_pin+remaining / 429 retryAfter /
// pin_not_set / cap_full / 网络异常），逐字口径见 describePairError。
// 文案硬编码中文（移动页 i18n 随 M3 完善）；P8f 双态配色与 Board.tsx 同口径。
export default function PairPage({ onPaired, probing = false }: PairPageProps) {
  const [pin, setPin] = useState("");
  const [status, setStatus] = useState<PairStatus>("idle");
  const [msg, setMsg] = useState(""); // 错误/提示行
  const [autoFill, setAutoFill] = useState(false); // 扫码路径：已自动填入提示
  // StrictMode 下 effect 会双触发，防扫码 pin 重复提交（重复提交会烧限速次数）
  const autoSubmitted = useRef(false);

  const submit = useCallback(
    async (raw: string) => {
      const p = raw.trim();
      // 扫码坏链防御（手输路径按钮已禁用，到不了这里）：不发注定失败的请求
      if (!/^\d{4}$/.test(p)) {
        setStatus("idle");
        setMsg("链接里的访问密码格式不对，请手动输入 4 位数字");
        return;
      }
      setStatus("pairing");
      setMsg("");
      try {
        const r = await pairWithPin(p);
        if (r.ok) {
          setStatus("ok");
          onPaired();
          return;
        }
      } catch (e) {
        setStatus("idle");
        setMsg(describePairError(e));
        return;
      }
      setStatus("idle");
      setMsg("配对失败，请重试");
    },
    [onPaired]
  );

  useEffect(() => {
    if (autoSubmitted.current) return;
    autoSubmitted.current = true;
    const hash = window.location.hash;
    if (!hash.startsWith("#pin=")) return;
    const p = decodeURIComponent(hash.slice("#pin=".length));
    // 提交前清掉 hash，避免刷新/回退重复提交（重复提交会烧限速次数）
    window.history.replaceState(null, "", window.location.pathname + window.location.search);
    setPin(p);
    setAutoFill(true);
    void submit(p);
  }, [submit]);

  return (
    <div className="flex min-h-screen flex-col items-center justify-center bg-white px-6 text-slate-800 dark:bg-slate-950 dark:text-slate-200">
      <div className="w-full max-w-xs">
        <h1 className="text-xl font-semibold text-slate-900 dark:text-slate-100">MAM 远程接入</h1>
        {/* M5 P3-b：探测期（paired===null）渲染连接指示器而非密码表单——区分
            「探测中」与「未配对」，隧道场景探测可达一二十秒；副标题随态切换 */}
        {probing ? (
          <>
            <p className="mt-1 text-sm text-slate-600 dark:text-slate-400">正在确认配对状态…</p>
            <div
              data-testid="probe-indicator"
              className="mt-8 flex items-center justify-center gap-2"
            >
              <span className="h-2.5 w-2.5 animate-pulse rounded-full bg-slate-400 dark:bg-slate-500" />
              <span className="text-sm text-slate-500 dark:text-slate-400">正在连接看板…</span>
            </div>
          </>
        ) : (
          <>
            <p className="mt-1 text-sm text-slate-600 dark:text-slate-400">
              输入访问密码，绑定此设备（180 天免输入）
            </p>
            <form
              onSubmit={(e) => {
                e.preventDefault();
                void submit(pin);
              }}
            >
              <input
                value={pin}
                onChange={(e) => {
                  setPin(e.target.value);
                  if (msg) setMsg("");
                }}
                placeholder="0000"
                type="password"
                inputMode="numeric"
                maxLength={4}
                autoComplete="off"
                autoCapitalize="off"
                spellCheck={false}
                data-testid="pin-input"
                className="mt-5 w-full rounded-lg border border-slate-300 bg-white py-2.5 text-center font-mono text-xl tracking-[10px] text-slate-900 focus:border-slate-500 focus:outline-none dark:border-slate-700 dark:bg-slate-800 dark:text-slate-100"
              />
              {autoFill && status === "pairing" && (
                <p
                  data-testid="autofill-hint"
                  className="mt-2 text-center text-xs text-emerald-600 dark:text-emerald-400"
                >
                  ✓ 已从链接自动填入，正在进入…
                </p>
              )}
              <button
                type="submit"
                disabled={pin.trim().length !== 4 || status === "pairing"}
                className="mt-4 w-full rounded-lg bg-slate-800 py-2.5 text-sm font-medium text-white disabled:opacity-50 dark:bg-slate-100 dark:text-slate-900"
              >
                {status === "pairing" ? "进入中…" : "进入看板"}
              </button>
            </form>
            {status === "idle" && msg && (
              <p data-testid="pair-error" className="mt-4 text-sm text-rose-700 dark:text-rose-400">
                {msg}
              </p>
            )}
          </>
        )}
        {status === "ok" && (
          <p className="mt-4 text-sm text-emerald-700 dark:text-emerald-400">
            接入成功，正在进入看板…
          </p>
        )}
        <p className="mt-10 text-center text-xs text-slate-500 dark:text-slate-400">
          扫码进入时无需手动输入
        </p>
      </div>
    </div>
  );
}

/** /pair/pin 错误分診（线稿口径）：剩余次数 / 锁定分钟 / 未设密码 / 设备满 / 网络。
 *  非 JSON 错误体（data=null）按状态码兜底；未知组合统一「配对失败」。 */
function describePairError(e: unknown): string {
  if (!(e instanceof ApiError) || e.status === null) {
    return "网络异常，请检查连接后重试";
  }
  const data = (e.data ?? {}) as { error?: string; remaining?: number; retryAfter?: number };
  if (data.error === "invalid_pin") {
    if (typeof data.remaining === "number" && data.remaining > 0) {
      return `密码不正确，还可尝试 ${data.remaining} 次（连续输错将锁定 10 分钟）`;
    }
    return "密码错误次数过多，已锁定，请约 10 分钟后再试";
  }
  if (e.status === 429 && typeof data.retryAfter === "number") {
    // 服务端给秒，人读分钟（向上取整，最少 1——「还有 40 秒」显示成 0 分钟是反直觉的）
    const mins = Math.max(1, Math.ceil(data.retryAfter / 60));
    return `尝试次数过多，已锁定，请约 ${mins} 分钟后再试`;
  }
  if (data.error === "pin_not_set") return "桌面端尚未设置访问密码";
  if (data.error === "cap_full") return "设备数量已达上限，请在桌面端花名册腾位后重试";
  return "配对失败，请重试";
}
