// 主题管理 — 无 Context 的事件驱动 store（修复：设置页深浅色切换失效）
//
// 背景 bug：原实现基于 React Context（WindowFrame 内挂 Provider），但设置页等懒加载
// 页面渲染时 useContext 解析到的是 createContext 的 no-op 默认值（Provider 与消费者
// 运行时分属两个 Context 对象），点击主题按钮静默无效。改为不依赖 Context 的全局
// store：localStorage 持久化 + 自定义事件（同窗口）+ storage 事件（跨窗口）同步，
// 对任何模块实例/Context 身份问题免疫，且模块加载即应用主题类（避免首帧闪白）。
import { useCallback, useEffect, useState } from "react";

export type Theme = "dark" | "light" | "system";

// 原 WindowFrame 使用的 storageKey（此前 setTheme 从未生效，无历史数据兼容负担）
const STORAGE_KEY = "tauri-ui-theme";
const THEME_EVENT = "mam-theme-changed";

function readStoredTheme(): Theme {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    return v === "dark" || v === "light" || v === "system" ? v : "system";
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

// 模块加载即应用（早于任何组件渲染）
applyThemeClass(readStoredTheme());

// 跟随系统模式下，系统主题变化时实时重应用
try {
  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    if (readStoredTheme() === "system") applyThemeClass("system");
  });
} catch {
  // 无 matchMedia：忽略
}

function writeTheme(theme: Theme) {
  try {
    localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    // ignore
  }
  applyThemeClass(theme);
  // 同窗口立即同步各订阅组件；跨窗口经 storage 事件回流
  window.dispatchEvent(new CustomEvent(THEME_EVENT));
}

/**
 * 主题 hook（API 与原 Context 版一致：{ theme, setTheme }）。
 * 任何组件可直接调用，无需 Provider 包裹。
 */
export function useTheme() {
  const [theme, setThemeState] = useState<Theme>(readStoredTheme);

  useEffect(() => {
    const sync = () => setThemeState(readStoredTheme());
    window.addEventListener(THEME_EVENT, sync);
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
