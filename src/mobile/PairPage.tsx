import { useCallback, useEffect, useRef, useState } from "react";
import { ApiError, confirmPairing, pair, pollPairing, requestPairing } from "./api";

interface PairPageProps {
  /** 配对成功后回调：App 重新探测并切换到已配对态 */
  onPaired: () => void;
}

type PairStatus = "idle" | "pairing" | "ok" | "error";

// M4 T2：请求接入子视图状态（idle=填设备名发起；waiting=轮询审批+4 位码确认；denied=预留）
type RequestState = "idle" | "waiting" | "denied";

// 请求接入区输入框：与 token 输入同款浅色/dark 双态类名（不含 font-mono）
const REQ_INPUT_CLS =
  "w-full rounded-lg border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 focus:border-slate-500 focus:outline-none dark:border-slate-700 dark:bg-slate-800 dark:text-slate-100";

// 请求接入区按钮：与主接入按钮同款
const REQ_BTN_CLS =
  "mt-3 w-full rounded-lg bg-slate-800 py-2 text-sm font-medium text-white disabled:opacity-50 dark:bg-slate-100 dark:text-slate-900";

// 极简配对卡：文案硬编码中文（移动页 i18n 随 M3 完善）。
// P8f 双态配色：全部颜色类为「浅色基准 + dark: 前缀」，与 Board.tsx 同口径
export default function PairPage({ onPaired }: PairPageProps) {
  const [token, setToken] = useState("");
  const [status, setStatus] = useState<PairStatus>("idle");
  // StrictMode 下 effect 会双触发，防扫码 token 重复提交
  const autoSubmitted = useRef(false);

  // M4 T2：请求接入流状态
  const [reqState, setReqState] = useState<RequestState>("idle");
  const [reqId, setReqId] = useState<string | null>(null);
  const [deviceName, setDeviceName] = useState("");
  const [code, setCode] = useState("");
  const [msg, setMsg] = useState(""); // 错误/提示行（设备已满/次数用尽/过期）

  const submit = useCallback(
    async (raw: string) => {
      const t = raw.trim();
      if (!t) return;
      setStatus("pairing");
      try {
        const r = await pair(t);
        if (r.ok) {
          setStatus("ok");
          onPaired();
          return;
        }
      } catch {
        // 网络异常与 403 同样落入错误文案
      }
      setStatus("error");
    },
    [onPaired]
  );

  useEffect(() => {
    if (autoSubmitted.current) return;
    autoSubmitted.current = true;
    const hash = window.location.hash;
    if (!hash.startsWith("#token=")) return;
    const t = hash.slice("#token=".length);
    // 提交前清掉 hash，避免刷新/回退重复提交过期 token
    window.history.replaceState(null, "", window.location.pathname + window.location.search);
    setToken(t);
    void submit(t);
  }, [submit]);

  // M4 T2：发起请求接入（设备名缺省兜底为「手机浏览器」）
  const startRequest = async () => {
    try {
      const r = await requestPairing(deviceName || "手机浏览器");
      setReqId(r.requestId);
      setReqState("waiting");
      setMsg("");
    } catch (e) {
      // 429 queue_full/ip_busy → 同一友好文案（不给队列状态预言机）
      setMsg(
        e instanceof ApiError && e.status === 429 ? "请求过多，请稍后再试" : "请求失败，请重试"
      );
    }
  };

  // M4 T2：4 位码确认（trim 后长度 4 才提交）
  const submitCode = async () => {
    if (!reqId || code.trim().length !== 4) return;
    const r = await confirmPairing(reqId, code.trim());
    if (r.ok) {
      onPaired();
      return;
    }
    if (r.error === "cap_full") setMsg("设备已满，请在桌面端花名册腾位后重试");
    else if (r.error === "exhausted") {
      setReqState("idle");
      setMsg("错误次数过多，请重新发起请求");
    } else if (r.error === "expired") {
      setReqState("idle");
      setMsg("请求已过期，请重新发起");
    } else setMsg(`确认码错误，剩余 ${r.triesLeft ?? 0} 次机会`);
  };

  // waiting 态轮询（3s；卸载即停）
  useEffect(() => {
    if (reqState !== "waiting" || !reqId) return;
    let stop = false;
    const tick = async () => {
      try {
        const s = await pollPairing(reqId!);
        if (stop) return;
        if (s === "approved") {
          onPaired();
          return;
        }
        if (s === "expired") {
          if (!stop) {
            setReqState("idle");
            setMsg("请求已过期（5 分钟），请重新发起");
          }
        }
      } catch {
        /* 网络抖动继续轮询 */
      }
    };
    void tick();
    const timer = setInterval(() => void tick(), 3000);
    return () => {
      stop = true;
      clearInterval(timer);
    };
  }, [reqState, reqId, onPaired]);

  return (
    <div className="flex min-h-screen items-center justify-center bg-white px-4 text-slate-800 dark:bg-slate-950 dark:text-slate-200">
      <div className="w-full max-w-sm rounded-2xl border border-slate-200 bg-slate-100 p-6 shadow-lg dark:border-transparent dark:bg-slate-900">
        <h1 className="text-xl font-semibold text-slate-900 dark:text-slate-100">MAM 远程接入</h1>
        <p className="mt-1 mb-6 text-sm text-slate-600 dark:text-slate-400">
          输入桌面端生成的接入码，绑定此设备
        </p>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void submit(token);
          }}
        >
          <input
            value={token}
            onChange={(e) => {
              setToken(e.target.value);
              if (status === "error") setStatus("idle");
            }}
            placeholder="接入码"
            autoComplete="off"
            autoCapitalize="off"
            spellCheck={false}
            className="w-full rounded-lg border border-slate-300 bg-white px-3 py-2 font-mono text-sm text-slate-900 focus:border-slate-500 focus:outline-none dark:border-slate-700 dark:bg-slate-800 dark:text-slate-100"
          />
          <button
            type="submit"
            disabled={!token.trim() || status === "pairing"}
            className="mt-3 w-full rounded-lg bg-slate-800 py-2 text-sm font-medium text-white disabled:opacity-50 dark:bg-slate-100 dark:text-slate-900"
          >
            {status === "pairing" ? "接入中…" : "接入"}
          </button>
        </form>
        {status === "error" && (
          <p className="mt-4 text-sm text-rose-700 dark:text-rose-400">
            接入码无效或已过期，请重新扫码或在桌面端重新生成
          </p>
        )}
        {status === "ok" && (
          <p className="mt-4 text-sm text-emerald-700 dark:text-emerald-400">
            接入成功，正在进入看板…
          </p>
        )}

        {/* M4 T2：请求接入区（主视图下方；idle=设备名发起 / waiting=轮询+4 位码确认） */}
        <div className="mt-6 border-t border-slate-200 pt-5 dark:border-slate-700">
          {reqState === "waiting" && reqId ? (
            <>
              <p className="text-sm text-slate-600 dark:text-slate-400">
                等待桌面批准…（5 分钟内有效）
              </p>
              <form
                onSubmit={(e) => {
                  e.preventDefault();
                  void submitCode();
                }}
              >
                <input
                  value={code}
                  onChange={(e) => setCode(e.target.value)}
                  placeholder="4 位确认码"
                  inputMode="numeric"
                  autoComplete="off"
                  autoCapitalize="off"
                  spellCheck={false}
                  className={REQ_INPUT_CLS + " mt-3"}
                />
                <button type="submit" disabled={code.trim().length !== 4} className={REQ_BTN_CLS}>
                  确认
                </button>
              </form>
            </>
          ) : (
            <>
              <p className="text-sm text-slate-600 dark:text-slate-400">
                没有接入码？填写设备名请求接入
              </p>
              <form
                onSubmit={(e) => {
                  e.preventDefault();
                  void startRequest();
                }}
              >
                <input
                  value={deviceName}
                  onChange={(e) => setDeviceName(e.target.value)}
                  placeholder="设备名称（选填）"
                  autoComplete="off"
                  autoCapitalize="off"
                  spellCheck={false}
                  className={REQ_INPUT_CLS + " mt-3"}
                />
                <button type="submit" className={REQ_BTN_CLS}>
                  请求接入
                </button>
              </form>
            </>
          )}
          {msg && <p className="mt-4 text-sm text-rose-700 dark:text-rose-400">{msg}</p>}
        </div>
      </div>
    </div>
  );
}
