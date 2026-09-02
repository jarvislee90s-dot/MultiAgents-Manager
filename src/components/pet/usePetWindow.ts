// 宠物窗口控制 — 尺寸/位置/穿透/物理（spec §4/§8）。IPC 调用一律 try/catch 静默降级（浏览器预览兼容）。
import { useCallback, useEffect, useRef } from "react";
import { currentMonitor, getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalPosition, LogicalSize } from "@tauri-apps/api/dpi";
import { loadPosition, savePosition } from "./petConfig";

export const GRAVITY = 1400; // px/s²（spec §8，原版同值）
export const DAMP = 0.86;
export const MIN_VX = 24;

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}
export interface FallState {
  x: number;
  y: number;
  vx: number;
  vy: number;
}
/** 工作区（逻辑像素，排除任务栏/Dock） */
export interface WorkArea {
  x: number;
  y: number;
  width: number;
  height: number;
}

export function bottomAnchoredY(oldY: number, oldH: number, newH: number): number {
  return oldY + oldH - newH;
}

export function clampToWorkArea(
  x: number,
  y: number,
  w: number,
  h: number,
  work: WorkArea
): { x: number; y: number } {
  return {
    x: Math.min(Math.max(x, work.x), work.x + work.width - w),
    y: Math.min(Math.max(y, work.y), work.y + work.height - h),
  };
}

export function hitTest(rects: Rect[], px: number, py: number): boolean {
  return rects.some((r) => px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h);
}

export function stepFall(
  s: FallState,
  dt: number,
  groundY: number
): FallState & { landed: boolean; rest: boolean } {
  // 半隐式欧拉积分：先更新速度，位移用「新速度 × dt」与「½gt²」等价需取平均速度修正，此处按恒加速度精确积分
  const vy = s.vy + GRAVITY * dt;
  let y = s.y + s.vy * dt + 0.5 * GRAVITY * dt * dt;
  const vx = s.vx * Math.pow(DAMP, dt * 60);
  const x = s.x + vx * dt;
  const landed = y >= groundY;
  if (landed) y = groundY;
  return { x, y, vx, vy, landed, rest: landed && Math.abs(vx) < MIN_VX };
}

async function getWorkArea(): Promise<WorkArea | null> {
  try {
    // Tauri 2 的 currentMonitor 是模块级函数（Window 实例上无此方法）
    const mon = await currentMonitor();
    if (!mon?.workArea) return null;
    const k = mon.scaleFactor || 1;
    const wa = mon.workArea; // 物理像素 → 逻辑
    return {
      x: wa.position.x / k,
      y: wa.position.y / k,
      width: wa.size.width / k,
      height: wa.size.height / k,
    };
  } catch {
    return null;
  }
}

/**
 * 窗口几何与穿透控制：
 * - registerInteractive 登记交互实体（精灵/卡片/菜单）；窗口常驻交互（穿透已按 spec §16 预案降级，见 onMove 说明）
 * - syncSize(w, h)：调用方在 useLayoutEffect 量测内容 DOM 后驱动窗口尺寸（防抖 50ms + 底部锚定，spec §4.2）
 * - beginDrag/releaseDrag 拖拽窗口与抛掷物理
 */
export function usePetWindow() {
  const contentRef = useRef<HTMLDivElement | null>(null);
  const interactiveEls = useRef(new Set<HTMLElement>());
  const dragRef = useRef<{
    pointerId: number;
    startX: number;
    startY: number;
    winX: number;
    winY: number;
    samples: { t: number; x: number; y: number }[];
  } | null>(null);
  const geoRef = useRef<{ x: number; y: number; w: number; h: number } | null>(null);
  const ignoringRef = useRef(false); // 窗口常驻交互（穿透禁用），见 onMove 处的降级说明
  const menuOpenRef = useRef(false);
  const fallRafRef = useRef(0);
  const resizeTimer = useRef<number | null>(null);

  const readGeometry = useCallback(async () => {
    try {
      const win = getCurrentWindow();
      const pos = await win.outerPosition();
      const size = await win.outerSize();
      const k = (await win.scaleFactor()) || 1;
      const geo = { x: pos.x / k, y: pos.y / k, w: size.width / k, h: size.height / k };
      geoRef.current = geo;
      return geo;
    } catch {
      return geoRef.current;
    }
  }, []);

  const setIgnoring = useCallback(async (ignore: boolean) => {
    if (ignoringRef.current === ignore) return;
    ignoringRef.current = ignore;
    try {
      // Tauri 2 的 setIgnoreCursorEvents(ignore) 无 options 参数；mousemove 转发由 Webview 自身事件处理（spec D11 意图）
      await getCurrentWindow().setIgnoreCursorEvents(ignore);
    } catch {
      // 浏览器预览/旧版本：跳过
    }
  }, []);

  /** 内容实测尺寸 → 窗口 setSize + 底部锚定 setPosition（防抖 50ms，spec §4.2） */
  const syncSize = useCallback(
    async (w: number, h: number) => {
      if (resizeTimer.current) window.clearTimeout(resizeTimer.current);
      resizeTimer.current = window.setTimeout(async () => {
        if (w <= 0 || h <= 0) return;
        try {
          const win = getCurrentWindow();
          const geo = (await readGeometry()) ?? { x: 0, y: 0, w, h };
          if (geo.w === w && geo.h === h) return;
          const work = await getWorkArea();
          let nx = geo.x;
          let ny = bottomAnchoredY(geo.y, geo.h, h); // 底部锚定：精灵不动
          if (work) ({ x: nx, y: ny } = clampToWorkArea(nx, ny, w, h, work));
          await win.setSize(new LogicalSize(w, h));
          await win.setPosition(new LogicalPosition(nx, ny));
          geoRef.current = { x: nx, y: ny, w, h };
        } catch {
          // ignore
        }
      }, 50);
    },
    [readGeometry]
  );

  // 启动：恢复记忆位置 + 初始置底
  useEffect(() => {
    (async () => {
      const saved = loadPosition();
      const geo = await readGeometry();
      if (!geo) return;
      if (saved) {
        const work = await getWorkArea();
        const target = work ? clampToWorkArea(saved.x, saved.y, geo.w, geo.h, work) : saved;
        try {
          await getCurrentWindow().setPosition(new LogicalPosition(target.x, target.y));
        } catch {
          /* ignore */
        }
      }
      await setIgnoring(false); // 显式落定交互态（重复调用经 ref 去重为 no-op）
    })();
  }, [readGeometry, setIgnoring]);

  // forward mousemove → 命中检测（spec §4.4；穿透切换已按 §16 预案降级，见 onMove 内注释）
  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (dragRef.current || menuOpenRef.current) {
        void setIgnoring(false);
        return;
      }
      const rects: Rect[] = [];
      const base = contentRef.current?.getBoundingClientRect();
      if (base) {
        for (const el of interactiveEls.current) {
          const r = el.getBoundingClientRect();
          rects.push({ x: r.x, y: r.y, w: r.width, h: r.height });
        }
        const hit = hitTest(rects, e.clientX, e.clientY);
        // Tauri 2.11 无 forward 选项，忽略态一旦生效事件流即断、无法悬停恢复（spec D11 风险命中），
        // 按 spec §16 预案降级为窗口常驻交互（穿透禁用），待 Tauri 提供 forward API 后恢复悬停切换。
        if (!hit) return; // 未命中（透明区）不切穿透，保持 ignore=false
        void setIgnoring(false);
      }
    };
    window.addEventListener("mousemove", onMove);
    return () => window.removeEventListener("mousemove", onMove);
  }, [setIgnoring]);

  const registerInteractive = useCallback((el: HTMLElement | null) => {
    if (el) interactiveEls.current.add(el);
    return () => {
      if (el) interactiveEls.current.delete(el);
    };
  }, []);

  const moveBy = useCallback(
    async (dx: number, dy: number) => {
      const geo = (await readGeometry()) ?? geoRef.current;
      if (!geo) return;
      const nx = geo.x + dx;
      const ny = geo.y + dy;
      geoRef.current = { ...geo, x: nx, y: ny };
      try {
        await getCurrentWindow().setPosition(new LogicalPosition(nx, ny));
      } catch {
        /* ignore */
      }
    },
    [readGeometry]
  );

  const beginDrag = useCallback(
    (e: React.PointerEvent) => {
      void readGeometry().then((geo) => {
        if (!geo) return;
        dragRef.current = {
          pointerId: e.pointerId,
          startX: e.clientX,
          startY: e.clientY,
          winX: geo.x,
          winY: geo.y,
          samples: [],
        };
      });
    },
    [readGeometry]
  );

  const trackDrag = useCallback((e: React.PointerEvent) => {
    const d = dragRef.current;
    if (!d || d.pointerId !== e.pointerId) return null;
    d.samples.push({ t: performance.now(), x: e.clientX, y: e.clientY });
    while (d.samples.length > 0 && performance.now() - d.samples[0].t > 150) d.samples.shift();
    return {
      dx: e.clientX - d.startX,
      dy: e.clientY - d.startY,
      movedX: e.clientX - (d.samples[0]?.x ?? e.clientX),
      movedY: e.clientY - (d.samples[0]?.y ?? e.clientY),
    };
  }, []);

  /** 松手：gravity 开→抛物坠落（rAF 循环 moveWindow）；否则停驻记忆（spec §8） */
  const releaseDrag = useCallback((opts: { gravity: boolean; onLand?: () => void }) => {
    const d = dragRef.current;
    dragRef.current = null;
    if (!d) return;
    const geo = geoRef.current;
    if (!geo) return;
    const px = d.samples.length >= 2 ? d.samples[d.samples.length - 1] : null;
    const first = d.samples[0] ?? null;
    const dt = px && first ? (px.t - first.t) / 1000 : 0;
    const vx0 = px && first && dt > 0 ? (px.x - first.x) / dt : 0;
    if (!opts.gravity || !px) {
      savePosition({ x: geo.x, y: geo.y });
      return;
    }
    let st: FallState = { x: geo.x, y: geo.y, vx: vx0, vy: 0 };
    let landedFired = false;
    let last = performance.now();
    const tick = async (t: number) => {
      const dts = Math.min(0.05, (t - last) / 1000);
      last = t;
      const work = await getWorkArea();
      const geoNow = geoRef.current;
      if (!geoNow) return;
      const groundY = work ? work.y + work.height - geoNow.h : st.y;
      const r = stepFall(st, dts, groundY);
      st = { x: r.x, y: r.y, vx: r.vx, vy: r.vy };
      geoRef.current = { ...geoNow, x: r.x, y: r.y };
      try {
        await getCurrentWindow().setPosition(new LogicalPosition(r.x, r.y));
      } catch {
        /* ignore */
      }
      if (r.landed && !landedFired) {
        landedFired = true;
        opts.onLand?.();
      }
      if (r.rest) {
        savePosition({ x: r.x, y: r.y });
        fallRafRef.current = 0;
        return;
      }
      fallRafRef.current = requestAnimationFrame(tick);
    };
    fallRafRef.current = requestAnimationFrame(tick);
  }, []);

  const setMenuOpen = useCallback(
    (open: boolean) => {
      menuOpenRef.current = open;
      if (open) void setIgnoring(false);
    },
    [setIgnoring]
  );

  // 卸载清理
  useEffect(
    () => () => {
      if (fallRafRef.current) cancelAnimationFrame(fallRafRef.current);
      if (resizeTimer.current) window.clearTimeout(resizeTimer.current);
    },
    []
  );

  return {
    contentRef,
    registerInteractive,
    syncSize,
    beginDrag,
    trackDrag,
    releaseDrag,
    moveBy,
    setMenuOpen,
    readGeometry,
  };
}
