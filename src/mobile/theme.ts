// P8f 日/夜双皮肤：默认跟随系统、手动切换持久化（localStorage）
//
// 与桌面端的刻意分叉：桌面走语义 CSS token（src/index.css 的 .dark 覆盖 CSS 变量），
// 不用 dark: 前缀类；移动端样式只有 tailwind 原子类，走 dark: 前缀双态类，因此
// mobile.css 必须用 @custom-variant 把 dark 变体声明为「类策略」——Tailwind v4 默认
// 是 media 查询（跟随系统），不加声明时本模块的 classList.toggle("dark") 对手动切换
// 完全无效（P8f 手动切换成死功能）。

export type Theme = "light" | "dark";

/** 持久化 key：须与 mobile.html 防闪白内联脚本读取的 key 严格一致（同一字符串常量两处使用） */
const KEY = "mam-theme";

/** 读已保存主题：localStorage 不可用（隐私模式 Safari 抛 SecurityError）或值非法时返回 null */
function readSaved(): Theme | null {
  try {
    const saved = localStorage.getItem(KEY);
    return saved === "light" || saved === "dark" ? saved : null;
  } catch {
    return null; // 读失败按「无已保存值」处理，降级到系统偏好分支
  }
}

/** 系统是否浅色偏好：matchMedia 缺失（jsdom / 旧 WebView）或抛错时按非浅色处理（回退 dark） */
function prefersLight(): boolean {
  try {
    return window.matchMedia("(prefers-color-scheme: light)").matches;
  } catch {
    return false;
  }
}

/** 初始主题优先级：已保存值 > 系统偏好 > 默认 dark */
export function getInitialTheme(): Theme {
  return readSaved() ?? (prefersLight() ? "light" : "dark");
}

/**
 * 应用初始主题：把「读初始主题 → 设置 documentElement 类」收敛到一处，供 main.tsx 调用。
 * 默认参数即 getInitialTheme()（也可显式传入已算好的主题）；幂等，可重复调用。
 * 防闪白由 mobile.html 内联脚本负责（首帧前执行），此处是权威应用点。
 */
export function applyInitialTheme(theme: Theme = getInitialTheme()): void {
  document.documentElement.classList.toggle("dark", theme === "dark");
}

/** 翻转主题：写 localStorage 持久化 + 切换 documentElement 的 dark 类 + 返回新主题 */
export function toggleTheme(): Theme {
  // next 从**当前生效态**推导（终审 Important 3）：旧实现从 getInitialTheme() 推导，
  // 写 localStorage 失败被吞后该函数恒回落系统偏好 → 连续点击恒产同一 next，
  // 单向锁死（用户永远切不回）。以 documentElement 实际类状态翻转，写失败也自愈
  const next: Theme = document.documentElement.classList.contains("dark") ? "light" : "dark";
  try {
    localStorage.setItem(KEY, next);
  } catch {
    // 写失败（隐私模式/配额）：持久化放弃，但 next 已按生效态推导、不依赖
    // localStorage 回读——本次会话内仍可继续往返切换，仅刷新后回落系统偏好
  }
  document.documentElement.classList.toggle("dark", next === "dark");
  return next;
}
