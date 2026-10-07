#!/usr/bin/env node
// =============================================================================
// dev-preflight.mjs —— `pnpm tauri:dev` 的**前置自检**（2026-10-07 事故修复）
// =============================================================================
//
// ## 为什么需要它
// 用户的原话：「这个事情以后怎么不再发生？我只想弄 `pnpm tauri:dev` 就好了。」
//
// 2026-10-07 的事故链是这样的：改依赖（摘掉 `@tauri-apps/plugin-global-shortcut`）⇒ Vite 的
// 预打包依赖缓存失效并重建 ⇒ **dev server 重建后不再应答**（进程活着、状态 `U`、
// 1420 端口 IPv4/IPv6 探活全部超时）⇒ WebView 请求 `index.html` 永远不完成 ⇒ **所有窗口空白**。
// 因为 MAM 的窗口都是 `transparent: true`，空白看起来就是「看板是透明的」「宠物不显示」——
// 两个症状一个根因，而**根因在 dev server，不在应用代码**。
//
// ## 它做两件事（都只在**明确信号**下动手，宁可不动也不误伤）
//  ① **清掉过期的预打包缓存**：`package.json` / `pnpm-lock.yaml` 比 `node_modules/.vite` 新
//     ⇒ 依赖变过了，下次启动反正要重建（而重建期可能不响应）。直接删掉，让它干净重建。
//  ② **抢救被卡死进程占用的 1420**：端口有监听但 HTTP 探活**超时** ⇒ 那是上一轮卡死的 vite
//     （本次事故就是它），杀掉继续。**探活正常则绝不插手** —— 那种情况交给 `strictPort` 报错，
//     语义与从前一致（不会把用户正常的第二个实例悄悄干掉）。
//
// 平台：① 跨平台；② 只在 macOS / Linux 生效（用 `lsof`），Windows 上打印提示后跳过。
// =============================================================================
import { execSync } from "node:child_process";
import { existsSync, rmSync, statSync } from "node:fs";
import { createConnection } from "node:net";
import path from "node:path";

const ROOT = path.resolve(import.meta.dirname, "..");
const VITE_CACHE = path.join(ROOT, "node_modules", ".vite");
const PORT = 1420;
const PROBE_TIMEOUT_MS = 1500;

const say = (msg) => console.log(`[dev-preflight] ${msg}`);

/** ① 依赖变过 ⇒ 预打包缓存过期 */
function cleanStaleDepCache() {
  if (!existsSync(VITE_CACHE)) return;
  const cacheAt = statSync(VITE_CACHE).mtimeMs;
  const deps = ["package.json", "pnpm-lock.yaml"]
    .map((f) => path.join(ROOT, f))
    .filter(existsSync)
    .map((f) => ({ f: path.basename(f), at: statSync(f).mtimeMs }))
    .filter((d) => d.at > cacheAt);
  if (deps.length === 0) return;
  rmSync(VITE_CACHE, { recursive: true, force: true });
  say(`依赖已变更（${deps.map((d) => d.f).join(" / ")} 比预打包缓存新）→ 已清理 node_modules/.vite`);
}

/** 探活：能拿到任何 HTTP 响应字节就算「活着」；超时/拒连算「没应答」 */
function probe(host) {
  return new Promise((resolve) => {
    const sock = createConnection({ host, port: PORT });
    let done = false;
    const finish = (v) => {
      if (done) return;
      done = true;
      sock.destroy();
      resolve(v);
    };
    sock.setTimeout(PROBE_TIMEOUT_MS);
    sock.on("connect", () => sock.write("GET / HTTP/1.0\r\nHost: localhost\r\n\r\n"));
    sock.on("data", () => finish("alive"));
    sock.on("timeout", () => finish("hung"));
    sock.on("error", (e) => finish(e.code === "ECONNREFUSED" ? "free" : "hung"));
  });
}

/** ② 端口被卡死进程占用 ⇒ 杀掉（只在探活明确超时时） */
async function rescueHungPort() {
  const v4 = await probe("127.0.0.1");
  const v6 = await probe("::1");
  if (!v4.startsWith("hung") && !v6.startsWith("hung")) {
    if (v4 === "alive" || v6 === "alive") {
      say(`注意：${PORT} 已有一个**正常应答**的服务在跑（交给 strictPort 处理，不插手）`);
    }
    return;
  }
  if (process.platform === "win32") {
    say(`${PORT} 端口有进程但不应答；Windows 请手动结束它（任务管理器找 node.exe）`);
    return;
  }
  let pids = "";
  try {
    pids = execSync(`lsof -ti tcp:${PORT} -sTCP:LISTEN`, { encoding: "utf8" }).trim();
  } catch {
    /* lsof 没找到就不管 */
  }
  if (!pids) {
    say(`${PORT} 端口无人应答但也查不到监听进程（可能正在退出），继续启动`);
    return;
  }
  for (const pid of pids.split("\n")) {
    try {
      process.kill(Number(pid), "SIGKILL");
      say(`上一个 dev server 卡死（探测无应答）→ 已结束 PID ${pid}`);
    } catch (e) {
      say(`结束 PID ${pid} 失败：${e.message}`);
    }
  }
}

cleanStaleDepCache();
await rescueHungPort();
