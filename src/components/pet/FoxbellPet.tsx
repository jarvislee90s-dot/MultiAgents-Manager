// FoxbellPet — 桌宠本体（spec §7/§8/§9）。Task 8：精灵 + 帧步进 + look 环顾 + 缩放；
// Task 9 追加交互（拖拽/物理/点击/菜单）；Task 10 追加卡片；Task 12 追加事件接线。
import { useEffect, useRef, useState } from "react";
import { ANIM, frameStyle, FRAME_H, FRAME_W, type PetAnimKey } from "./petAnimations";
import { loadConfig, subscribeConfig, type PetConfig } from "./petConfig";
import { usePetWindow } from "./usePetWindow";

export function FoxbellPet() {
  const [cfg, setCfg] = useState<PetConfig>(() => loadConfig());
  const cfgRef = useRef(cfg);
  // 渲染期禁止写 ref（react-hooks/refs）：改在 effect 中同步，供 Task 9 交互读取最新配置
  useEffect(() => {
    cfgRef.current = cfg;
  }, [cfg]);
  useEffect(() => subscribeConfig(() => setCfg(loadConfig())), []);

  const pet = usePetWindow();
  const { registerInteractive, contentRef } = pet; // useCallback/useRef 稳定引用，避免依赖整个 pet 对象（每次渲染重建）
  const spriteRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => registerInteractive(spriteRef.current), [registerInteractive]);

  // ---- 动画状态机（spec §7：拖拽 > 瞬时 > 任务态 > look > idle）----
  const [anim, setAnim] = useState<PetAnimKey>("idle");
  const [frame, setFrame] = useState(0);
  const [lookFrame, setLookFrame] = useState(-1);
  const animRef = useRef<PetAnimKey>("idle");
  const frameRef = useRef(0);
  const stepTimer = useRef<number | null>(null);
  const stateRef = useRef<{ drag: PetAnimKey | null; transient: PetAnimKey | null; task: PetAnimKey | null; look: boolean }>({
    drag: null, transient: null, task: null, look: false,
  });
  const lookStop = useRef<(() => void) | null>(null);
  const genRef = useRef({ transient: 0, look: 0 });

  const later = useRef((fn: () => void, ms: number) => {
    const id = window.setTimeout(fn, ms);
    return () => window.clearTimeout(id);
  }).current;

  const cancelStep = () => {
    if (stepTimer.current !== null) {
      window.clearTimeout(stepTimer.current);
      stepTimer.current = null;
    }
  };

  const stepLoop = () => {
    const def = ANIM[(animRef.current === "look" ? "idle" : animRef.current) as keyof typeof ANIM];
    const i = frameRef.current;
    const ms = def.d[i] ?? 160;
    stepTimer.current = window.setTimeout(() => {
      frameRef.current = (i + 1) % def.d.length;
      setFrame(frameRef.current);
      stepLoop();
    }, ms);
  };

  const applyAnim = (key: PetAnimKey) => {
    if (animRef.current === key) return;
    animRef.current = key;
    setAnim(key);
    cancelStep();
    frameRef.current = 0;
    setFrame(0);
    stepLoop();
  };

  const refreshAnim = () => {
    const s = stateRef.current;
    applyAnim(s.drag ?? s.transient ?? s.task ?? (s.look ? "look" : "idle"));
  };

  /** 瞬时动作（代数计数防过期覆盖，spec F4）—— Task 9 交互接线时启用 */
  const playTransient = useRef((key: PetAnimKey, ms: number) => {
    const gen = ++genRef.current.transient;
    stateRef.current.transient = key;
    refreshAnim();
    later(() => {
      if (genRef.current.transient === gen && stateRef.current.transient === key) {
        stateRef.current.transient = null;
        refreshAnim();
      }
    }, ms);
  }).current;
  // 骨架阶段尚未接线（Task 9 使用）：显式引用以满足 tsc noUnusedLocals
  void playTransient;

  // ---- look 环顾：空闲 6s 触发，16 向 250ms/帧，任何状态打断（spec F2）----
  const stopLook = () => {
    lookStop.current?.();
    lookStop.current = null;
    if (stateRef.current.look) {
      stateRef.current.look = false;
      setLookFrame(-1);
    }
  };
  const scheduleNextLook = () => {
    const gen = ++genRef.current.look;
    later(() => {
      if (genRef.current.look !== gen) return;
      const s = stateRef.current;
      if (!s.drag && !s.transient && !s.task) {
        s.look = true;
        setLookFrame(0);
        refreshAnim();
        let i = 0;
        const id = window.setInterval(() => {
          i += 1;
          if (i >= 16) {
            window.clearInterval(id);
            lookStop.current = null;
            stopLook();
            refreshAnim();
            scheduleNextLook();
          } else {
            setLookFrame(i);
          }
        }, 250);
        lookStop.current = () => window.clearInterval(id);
      } else {
        scheduleNextLook();
      }
    }, 6000);
  };

  useEffect(() => {
    stepLoop();
    scheduleNextLook();
    return () => {
      cancelStep();
      stopLook();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const px = (v: number) => Math.round(v * cfg.scale);
  const style = frameStyle(anim, frame, lookFrame, cfg.scale);

  return (
    <div ref={contentRef} style={{ position: "fixed", inset: 0, overflow: "visible" }}>
      <div
        ref={spriteRef}
        data-testid="pet-sprite"
        style={{
          position: "absolute",
          left: "50%",
          transform: "translateX(-50%)",
          bottom: px(50), // 底部气泡区（spec §4.2）
          width: px(FRAME_W),
          height: px(FRAME_H),
          backgroundImage: "url(/pet/spritesheet.webp)",
          backgroundPosition: style.backgroundPosition,
          backgroundSize: style.backgroundSize,
          backgroundRepeat: "no-repeat",
          cursor: "grab",
          touchAction: "none",
          userSelect: "none",
        }}
      />
    </div>
  );
}
