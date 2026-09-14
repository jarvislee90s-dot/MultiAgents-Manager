import { useCallback, useEffect, useRef, useState } from "react";
import { pair } from "./api";

interface PairPageProps {
  /** 配对成功后回调：App 重新探测并切换到已配对态 */
  onPaired: () => void;
}

type PairStatus = "idle" | "pairing" | "ok" | "error";

// 极简暗色配对卡：文案硬编码中文（移动页 i18n 随 M3 完善）
export default function PairPage({ onPaired }: PairPageProps) {
  const [token, setToken] = useState("");
  const [status, setStatus] = useState<PairStatus>("idle");
  // StrictMode 下 effect 会双触发，防扫码 token 重复提交
  const autoSubmitted = useRef(false);

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

  return (
    <div className="flex min-h-screen items-center justify-center bg-slate-950 px-4">
      <div className="w-full max-w-sm rounded-2xl bg-slate-900 p-6 shadow-lg">
        <h1 className="text-xl font-semibold text-slate-100">MAM 远程接入</h1>
        <p className="mt-1 mb-6 text-sm text-slate-400">输入桌面端生成的接入码，绑定此设备</p>
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
            className="w-full rounded-lg border border-slate-700 bg-slate-800 px-3 py-2 font-mono text-sm text-slate-100 focus:border-slate-500 focus:outline-none"
          />
          <button
            type="submit"
            disabled={!token.trim() || status === "pairing"}
            className="mt-3 w-full rounded-lg bg-slate-100 py-2 text-sm font-medium text-slate-900 disabled:opacity-50"
          >
            {status === "pairing" ? "接入中…" : "接入"}
          </button>
        </form>
        {status === "error" && (
          <p className="mt-4 text-sm text-rose-400">
            接入码无效或已过期，请重新扫码或在桌面端重新生成
          </p>
        )}
        {status === "ok" && (
          <p className="mt-4 text-sm text-emerald-400">接入成功，正在进入看板…</p>
        )}
      </div>
    </div>
  );
}
