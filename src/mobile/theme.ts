// P8f 日/夜双皮肤：默认跟随系统、手动切换持久化（localStorage）
//
// 与桌面端的刻意分叉：桌面走语义 CSS token（src/index.css 的 .dark 覆盖 CSS 变量），
// 不用 dark: 前缀类；移动端样式只有 tailwind 原子类，走 dark: 前缀双态类，因此
// mobile.css 必须用 @custom-variant 把 dark 变体声明为「类策略」——Tailwind v4 默认
// 是 media 查询（跟随系统），不加声明时本模块的 classList.toggle("dark") 对手动切换
// 完全无效（P8f 手动切换成死功能）。

export type Theme = "light" | "dark";

/** 持久化 key：须与 mobile.html 防闪白内联脚本读取的 key 严格一致（同一字符串常量两处使用） */
const KEY = "tuvis-theme";

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

// ============================================================
// 远程端外观配置（2026-10-05 UI 改版，spec §5/§6.2）：桌面外观配置器保存后经
// /m/api/v1/ui-config 下发；本模块把它应用到 documentElement 的 data 属性
// （mobile.css 六皮肤/字体/圆角表按属性切换），并维护 localStorage 镜像
// （mobile.html 防闪脚本首帧前读取同款值——服务器/镜像/默认三级回退）。
// 双层字体制度：这里只切 --font-ui 档位；--font-mono 恒定不随档位（见台账）
// ============================================================

export interface UiConfig {
  /** 白天皮肤（lpaper | lpure | lmist） */
  daySkin: string;
  /** 夜间皮肤（npaper | npure | ndeep） */
  nightSkin: string;
  /** 字体气质（std | term | round | serif） */
  font: string;
  /** 卡片圆角档位（8 | 12 | 18 | 22） */
  radius: number;
  /** 品牌点缀（edge | top | tint | none） */
  accent: string;
}

const UI_KEY = "mam-ui-config";

// 当前生效配置（null = 无配置 → data 属性缺省 = mobile.css 默认纸感对/标准/12px）
let activeUiConfig: UiConfig | null = null;

/** 按当前日夜模式把配置写到 documentElement 的 data 属性。日夜切换（toggleTheme）
 *  后必须重跑——白天/夜间用的是配置里的两张不同皮肤表 */
function applyUiAttrs(dark: boolean): void {
  const cfg = activeUiConfig;
  if (!cfg) return;
  const el = document.documentElement;
  el.dataset.skin = dark ? cfg.nightSkin : cfg.daySkin;
  el.dataset.fontTheme = cfg.font;
  el.dataset.radius = String(cfg.radius);
}

/** 读取 localStorage 镜像（防闪脚本同 key 严格一致）；解析失败/结构非法 → null。
 *  Partial 校验：daySkin/nightSkin 缺失即视为无效（镜像不完整宁用默认，不猜） */
function readSavedUiConfig(): UiConfig | null {
  try {
    const raw = localStorage.getItem(UI_KEY);
    if (!raw) return null;
    const v = JSON.parse(raw) as Partial<UiConfig>;
    if (typeof v.daySkin !== "string" || typeof v.nightSkin !== "string") return null;
    return {
      daySkin: v.daySkin,
      nightSkin: v.nightSkin,
      font: typeof v.font === "string" ? v.font : "std",
      radius: typeof v.radius === "number" ? v.radius : 12,
      accent: typeof v.accent === "string" ? v.accent : "edge",
    };
  } catch {
    return null; // 隐私模式/坏 JSON：与主题同口径，按「无镜像」处理
  }
}

/** 应用配置（服务器值或镜像）：写镜像 + 记 active + 按当前模式设属性。幂等。
 *  cfg=null（服务器 403/失败回退口径）：清 active、保留现状属性——不闪变不假清 */
export function applyUiConfig(cfg: UiConfig | null): void {
  if (!cfg) {
    activeUiConfig = null;
    return;
  }
  activeUiConfig = cfg;
  try {
    localStorage.setItem(UI_KEY, JSON.stringify(cfg));
  } catch {
    // 隐私模式/配额：持久化放弃，本会话仍生效（与主题 toggle 同口径）
  }
  applyUiAttrs(document.documentElement.classList.contains("dark"));
}

/** 初始应用：mobile.html 防闪脚本已按镜像先行（首帧前）；此处为权威应用点
 *  （与 applyInitialTheme 同刻执行），随后 main 流程用服务器值覆写 */
export function applyInitialUiConfig(): void {
  const saved = readSavedUiConfig();
  if (saved) applyUiConfig(saved);
}

/** 品牌点缀方式（Board 消费）：edge=左彩檐 / top=顶部细线 / tint=淡底渲染 /
 *  none=无檐。无配置 → edge（默认口径） */
export function currentAccent(): string {
  return activeUiConfig?.accent ?? "edge";
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
  // 日夜切换后重设皮肤属性：白天/夜间消费配置里的两张不同皮肤表
  applyUiAttrs(next === "dark");
  return next;
}
