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
  // dragTo 发送合并（in-flight 合并）：同一时刻仅一个 setPosition 在飞，见 dragTo 注释
  const dragInFlightRef = useRef(false);
  const dragTargetRef = useRef<{ x: number; y: number } | null>(null);
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
      // Tauri 2 的 setIgnoreCursorEvents(ignore) 无 forward 选项；当前恒为 false（整窗常驻交互，穿透禁用）。
      // true 分支仅为将来恢复穿透保留：恢复路径是 cursorPosition() 轮询 + hitTest（见 onMove 注释）。
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

  // 命中检测（spec §4.4 原设计；穿透当前禁用，本监听实际为 no-op，保留作恢复路径）
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
        // Tauri 2.11 无 forward 选项：忽略态一旦生效 webview 收不到任何鼠标事件（含 mousemove），
        // 事件流即断、无法悬停恢复——§16 的"静态划分"预案同样不可行（setIgnoreCursorEvents 是整窗开关）。
        // 实际采用：整窗常驻交互、穿透禁用（ignoringRef 恒 false，setIgnoring(true) 不可达）。
        // 恢复路径：@tauri-apps/api 2.11 的模块级 cursorPosition() 轮询（~30Hz）+ 下方 hitTest，
        // 或 Tauri 提供 forward 选项后本监听即可直接生效。
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

  /** 铆钉式拖动定位：以 beginDrag 记录的按下时刻窗口位置 + 屏幕绝对位移直接定位。
   *  无逐帧几何读取（旧 moveBy 每帧 3 次 IPC 且受 setPosition 落地竞态影响）；
   *  geoRef 乐观更新，拖动结束时 releaseDrag 仍按最新值落定。
   *  发送合并：同一时刻仅允许一个 setPosition 在飞，飞行期间的新位置只记最新值，
   *  落地后立即补发。快速反向拖拽时 pointermove 事件频率可达每秒数百个，逐个
   *  发 IPC 会在 WebView→Rust 队列里积压（实测问题 2：全程按住拖第二段方向时
   *  窗口慢约 1s 才"闪现"到位）；合并后队列最多排 1 个，始终追最新目标。 */
  const dragTo = useCallback((dx: number, dy: number) => {
    const d = dragRef.current;
    if (!d) return;
    const nx = d.winX + dx;
    const ny = d.winY + dy;
    geoRef.current = { x: nx, y: ny, w: geoRef.current?.w ?? 0, h: geoRef.current?.h ?? 0 };
    dragTargetRef.current = { x: nx, y: ny };
    if (dragInFlightRef.current) return;
    dragInFlightRef.current = true;
    const flush = (target: { x: number; y: number }) => {
      try {
        // setPosition 正常返回 Promise；但 mock/降级环境可能返回 undefined，统一吞掉
        void Promise.resolve(
          getCurrentWindow().setPosition(new LogicalPosition(target.x, target.y))
        )
          .catch(() => {
            /* ignore */
          })
          .then(() => {
            // 上一发落地：若期间又有新目标则立即补发，否则清空在飞位
            dragInFlightRef.current = false;
            const next = dragTargetRef.current;
            if (next && (next.x !== target.x || next.y !== target.y)) {
              dragInFlightRef.current = true;
              flush(next);
            } else {
              dragTargetRef.current = null;
            }
          });
      } catch {
        dragInFlightRef.current = false;
      }
    };
    flush({ x: nx, y: ny });
  }, []);

  const beginDrag = useCallback(
    (e: React.PointerEvent) => {
      // 铆钉从按下点建立：pointer capture 保证指针离开精灵后 move/up 事件仍派发给
      // 精灵（松手信号不丢）；屏幕绝对坐标增量不受窗口自身移动影响（无反馈循环）。
      // 中断在途的坠落物理：松手后的 rAF 坠落循环若不取消，会与新一轮 dragTo
      // 每帧互相覆盖窗口位置（实测问题 2：第二次拖拽不跟手、约 1s 后才"闪现"）
      if (fallRafRef.current) {
        cancelAnimationFrame(fallRafRef.current);
        fallRafRef.current = 0;
      }
      // 同步取 geoRef 缓存建铆钉（立即生效，按下后首帧 move 不丢）；异步 readGeometry
      // 仅在缓存缺失时兜底。jsdom 无 setPointerCapture：特性检测跳过（真机 WebView 有）
      (e.target as HTMLElement).setPointerCapture?.(e.pointerId);
      const cached = geoRef.current;
      if (cached) {
        dragRef.current = {
          pointerId: e.pointerId,
          startX: e.screenX,
          startY: e.screenY,
          winX: cached.x,
          winY: cached.y,
          samples: [],
        };
        return;
      }
      void readGeometry().then((geo) => {
        if (!geo) return;
        dragRef.current = {
          pointerId: e.pointerId,
          startX: e.screenX,
          startY: e.screenY,
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
    d.samples.push({ t: performance.now(), x: e.screenX, y: e.screenY });
    while (d.samples.length > 0 && performance.now() - d.samples[0].t > 150) d.samples.shift();
    return {
      // 铆钉式：窗口位置 = 按下时窗口位置 + 屏幕指针位移（绝对，无累积误差）
      dx: e.screenX - d.startX,
      dy: e.screenY - d.startY,
      movedX: e.screenX - (d.samples[0]?.x ?? e.screenX),
      movedY: e.screenY - (d.samples[0]?.y ?? e.screenY),
    };
  }, []);

  /** 松手：gravity 开→抛物坠落（rAF 循环 moveWindow）；否则停驻记忆（spec §8） */
  const releaseDrag = useCallback((opts: { gravity: boolean; onLand?: () => void }) => {
    const d = dragRef.current;
    dragRef.current = null;
    // 清掉 dragTo 的待发目标：在飞的最后一发落地后链路自然终止，
    // 不再补发（geoRef 已是最新目标位，坠落从该位起算）
    dragTargetRef.current = null;
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
      // 新拖拽已把本循环取消（beginDrag cancelAnimationFrame + 清零）：立即退出，
      // 不再续期。没有本检查时，cancel 后在途 tick 仍会 requestAnimationFrame 续期复活
      if (fallRafRef.current === 0) return;
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
    dragTo,
    setMenuOpen,
    readGeometry,
  };
}
