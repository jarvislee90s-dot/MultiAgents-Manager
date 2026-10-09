import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "path";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

/**
 * 监听地址：**显式绑 IPv4 回环**（2026-10-07 事故修复）。
 *
 * 为什么不能留 `false`（= Vite 默认「只监听 localhost」）：`localhost` 的解析结果取决于
 * 运行时的 DNS 顺序，实测本机 Node 26 把它解析成 **`::1`（IPv6）**⇒ dev server **只绑 IPv6**。
 * 而 Tauri WebView 请求的是 `http://localhost:1420`，一旦它按 IPv4 解析，就会**连不上** ——
 * 表现是窗口**全空白**（页面加载不出来 ⇒ 没有 `bg-background` ⇒ 因为是 `transparent: true`
 * 的窗口，看起来就是「透明的看板 / 看不见的桌宠」）。
 *
 * `TAURI_DEV_HOST` 若被显式设置（真机/局域网调试、移动端）**仍然优先**，行为不变。
 */
// @ts-expect-error process is a nodejs global
const devHost = host ?? "127.0.0.1";

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],

  resolve: {
    alias: {
      // `__dirname` → `import.meta.dirname`（2026-10-07）：Vite 8 的 `configLoader: 'native'`
      // 会对 `__dirname` 发一条每次启动都出现的弃用警告；本仓要求 Node ≥20.11（ESM 下有
      // `import.meta.dirname`）⇒ 换掉，让 `pnpm tauri:dev` 的输出干净。
      "@": path.resolve(import.meta.dirname, "./src"),
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: devHost,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      //
      // 2026-10-07 追加 `research/**` 与 `promo-materials/**`：前者是本仓的**调研参考库**
      // （里面躺着整份第三方前端仓库，实测 `research/refs/cc-switch/` 与
      // `research/refs/CodexPlusPlus/docs/` 各带一个 `index.html`），后者是宣传物料。
      // 两者都不是本应用的源码，却被 dev server 当成工程内容盯着 —— 既白烧文件监听，
      // 又（更要命）把它们的 HTML 拖进依赖扫描（见下面 `optimizeDeps.entries` 的说明）。
      ignored: ["**/src-tauri/**", "**/research/**", "**/promo-materials/**"],
    },
  },

  /**
   * 依赖预打包的**扫描入口**：只认本应用自己的 `index.html`。
   *
   * 为什么必须写死（2026-10-07 事故的真正根因）：Vite 的 `optimizeDeps.entries` **默认是
   * `**\/*.html`** —— 它会扫全仓的 HTML 入口。本仓的 `research/refs/` 下有两份第三方前端仓库，
   * 各带 `index.html` ⇒ 扫描器爬进它们的源码，撞上**没装**的依赖
   * （`@tauri-apps/plugin-log` / `framer-motion`）⇒ **整次依赖扫描失败**：
   *
   *     (!) Failed to run dependency scan. Skipping dependency pre-bundling. Error: The following
   *         dependencies are imported but could not be resolved: ...
   *
   * 后果不是「少优化了几个包」，而是**启动被拖长到近两分钟并在这段时间里不应答**
   * （实测 `scanning dependencies...` 11:09:54 → `dependencies optimized` 11:11:41），随后又一次
   * `optimized dependencies changed. reloading`。而 兔维斯 的所有窗口都是 `transparent: true`，
   * 页面加载不出来 ⇒ **看板看起来「透明」、桌宠「不显示」** —— 两个症状、一个根因，
   * 且根因在 dev server，不在应用代码。
   *
   * 写死成 `["index.html"]` 之后：扫描面只剩本应用 ⇒ 快、且不会再因为参考库而失败。
   * 新增真正的 HTML 入口（多页应用）时**必须同步加进这个数组**。
   */
  optimizeDeps: {
    entries: ["index.html"],
  },
}));
