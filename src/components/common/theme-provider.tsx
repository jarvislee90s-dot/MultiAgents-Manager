// 主题管理 — 全局 SSOT（DB settings KV）+ 模块级初始化 + 事件驱动（issue #3）
//
// 背景 bug：原实现以 localStorage 为事实源，跨窗口同步依赖 storage 事件；但 Tauri
// 各窗口是独立 WebView，storage 事件不跨窗口——设置窗口（独立子窗口）永远读不到
// 主窗口写入的 localStorage，恒为浅色。修复：主题事实源上移为 DB settings KV
// （ui_theme，Rust set_theme 命令写库并广播全局事件 mam-theme-changed）。
// 所有窗口（主窗口/设置窗口/其他子窗口）加载同一 SPA：模块加载时应用首帧缓存 +
// 模块级 initTheme() 从 DB 拉取当前主题校正 + 订阅全局事件实时跟随——无论从哪个
// 窗口切换，都写同一 SSOT、广播同一事件。localStorage 降级为首帧缓存（消除闪白）。
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Theme = "dark" | "light" | "system";

// 主题 SSOT 的 DB settings key（Rust set_theme 命令读写同一 key）
const THEME_SETTING_KEY = "ui_theme";
// 全局主题变更事件（Rust set_theme 广播；前端各窗口订阅）
const THEME_EVENT = "mam-theme-changed";
// 首帧缓存 key（localStorage 降级：仅作启动首帧缓存，非事实源）
const STORAGE_KEY = "tauri-ui-theme";
// 模块加载后用户是否已手动切换过主题：若已切换（缓存+广播已生效且为最新意图），
// 过期的 DB 拉取结果不得覆盖——消除「首帧拉取与用户切换」的窄竞态
let dirtySinceLoad = false;

function isTheme(v: unknown): v is Theme {
  return v === "dark" || v === "light" || v === "system";
}

function readCachedTheme(): Theme {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    return isTheme(v) ? v : "system";
  } catch {
    return "system";
  }
}

function prefersDark(): boolean {
  try {
    return window.matchMedia("(prefers-color-scheme: dark)").matches;
  } catch {
    return false; // jsdom 等环境无 matchMedia
  }
}

function applyThemeClass(theme: Theme) {
  const root = window.document.documentElement;
  root.classList.remove("light", "dark");
  root.classList.add(theme === "system" ? (prefersDark() ? "dark" : "light") : theme);
}

function writeCache(theme: Theme) {
  try {
    localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    // ignore
  }
}

/**
 * 模块级初始化：从 DB 事实源拉取当前主题并应用类/写回缓存。
 * 每个窗口（独立 WebView、独立模块实例）加载时由 main.tsx 显式调用触发执行，
 * 覆盖全部窗口——包括不消费 useTheme 的窗口（关于/通知等）。幂等：无 DB 记录或
 * 非合法值时保持首帧缓存不变；用户已手动切换过（dirtySinceLoad）则跳过过期 DB 值。
 */
export async function initTheme(): Promise<void> {
  try {
    const v = await invoke<string | null>("get_setting", { key: THEME_SETTING_KEY });
    if (!isTheme(v) || dirtySinceLoad) return;
    writeCache(v);
    applyThemeClass(v);
    // 通知已挂载的 useTheme 消费者同步状态（挂载早于校正完成的场景）
    window.dispatchEvent(new CustomEvent(THEME_EVENT));
  } catch {
    // 非 Tauri 环境（浏览器/mock）：忽略
  }
}

// 模块加载即应用缓存主题（早于任何组件渲染，消除首帧闪白）；DB 事实源随后校正
applyThemeClass(readCachedTheme());

// 模块级初始化：DB 拉取校正（对每个窗口生效，见 initTheme 注释）
void initTheme();

// 跟随系统模式下，系统主题变化时实时重应用
try {
  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    if (readCachedTheme() === "system") applyThemeClass("system");
  });
} catch {
  // 无 matchMedia：忽略
}

// 订阅全局主题事件（Rust set_theme 广播）：任何窗口切换主题，本窗口实时跟随。
// 监听器为模块级、随窗口生命周期存活（窗口销毁即释放，无需手动 unlisten）。
// 非 Tauri 环境（浏览器/mock）listen 返回 rejected promise → catch 静默降级为
// 仅同窗口自定义事件（listen 是 async 函数，同步 throw 也会转为 rejection）
void listen<{ theme: string }>(THEME_EVENT, (event) => {
  if (isTheme(event.payload?.theme)) {
    writeCache(event.payload.theme);
    applyThemeClass(event.payload.theme);
    // 同窗口各订阅组件经自定义事件同步
    window.dispatchEvent(new CustomEvent(THEME_EVENT));
  }
}).catch(() => {
  // 非 Tauri 环境：忽略
});

function writeTheme(theme: Theme) {
  dirtySinceLoad = true;
  writeCache(theme);
  applyThemeClass(theme);
  // 同窗口立即同步各订阅组件；跨窗口经 Rust 广播的全局事件回流
  window.dispatchEvent(new CustomEvent(THEME_EVENT));
  // 写 DB 事实源 + 广播全局事件（Rust set_theme 命令）；失败时缓存已生效，不阻塞 UI
  invoke("set_theme", { theme }).catch(() => {});
}

/**
 * 主题 hook（API 与原实现一致：{ theme, setTheme }）。
 * 任何组件可直接调用，无需 Provider 包裹。初始状态读首帧缓存；模块级 initTheme
 * 已完成 DB 校正（或校正完成后经自定义事件同步），无需在 hook 内重复拉取。
 */
export function useTheme() {
  const [theme, setThemeState] = useState<Theme>(readCachedTheme);

  useEffect(() => {
    const sync = () => setThemeState(readCachedTheme());
    window.addEventListener(THEME_EVENT, sync);
    // storage 事件仅浏览器多标签页场景有意义（Tauri 各窗口是独立 WebView，不互通）
    window.addEventListener("storage", sync);
    return () => {
      window.removeEventListener(THEME_EVENT, sync);
      window.removeEventListener("storage", sync);
    };
  }, []);

  const setTheme = useCallback((t: Theme) => {
    writeTheme(t);
    setThemeState(t);
  }, []);

  return { theme, setTheme };
}
