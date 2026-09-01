// FoxbellPet — 桌宠本体（spec §7/§8/§9）。Task 8：精灵 + 帧步进 + look 环顾 + 缩放；
// Task 9：指针交互（拖拽方向动画/松手物理/单击/双击）+ 语音字幕；Task 10：状态卡片 + 跳转/歧义候选；Task 12 追加事件接线。
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import type { Session } from "@/types/session";
import { JumpWindowCandidate } from "@/hooks/useSessionJump";
import { useSessionsQuery } from "@/lib/query/queries/sessions";
import { ANIM, frameStyle, FRAME_H, FRAME_W, type PetAnimKey } from "./petAnimations";
import { loadConfig, subscribeConfig, type PetConfig } from "./petConfig";
import { ackDone, cardsFromState, computePetStatus, type PetCard, type PetStatusState } from "./petStatus";
import { MIN_SPEECH_MS, parseManifest, VoicePlayer, type VoiceGroup } from "./petVoices";
import { usePetWindow } from "./usePetWindow";

export function FoxbellPet() {
  const { t } = useTranslation();
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
  const stateRef = useRef<{
    drag: PetAnimKey | null;
    transient: PetAnimKey | null;
    task: PetAnimKey | null;
    look: boolean;
  }>({
    drag: null,
    transient: null,
    task: null,
    look: false,
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
    // 离开 look（更高优先级状态抢占）时完整停掉扫视：interval + 状态位 + 帧号复位（Task 8 评审遗留）
    if (animRef.current !== "look") stopLook();
    frameRef.current = 0;
    setFrame(0);
    stepLoop();
  };

  const refreshAnim = () => {
    const s = stateRef.current;
    applyAnim(s.drag ?? s.transient ?? s.task ?? (s.look ? "look" : "idle"));
  };

  /** 瞬时动作（代数计数防过期覆盖，spec F4） */
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

  // ---- 语音与字幕（Task 12 事件接线复用，spec §6.2）----
  const [subtitle, setSubtitle] = useState<string | null>(null);
  const bubbleGen = useRef(0);
  const voiceRef = useRef<VoicePlayer | null>(null);
  const unlockedRef = useRef(false);

  // manifest 拉取一次；素材缺失/解析失败 → 语音静默降级，动作与字幕不受影响（spec §13）。
  // StrictMode 双挂载：卸载时丢弃过期响应，保证重挂载后仍会重新拉取
  useEffect(() => {
    let cancelled = false;
    fetch("/pet/manifest.json")
      .then((r) => r.json())
      .then((raw) => {
        if (cancelled) return;
        const entries = parseManifest(raw);
        const player = new VoicePlayer();
        player.load(entries);
        voiceRef.current = player;
        // E6 竞态修复：手势解锁先于 manifest 就绪时，补一次 unlock，避免播放器解锁位丢失
        if (unlockedRef.current) player.unlock();
      })
      .catch(() => {
        /* 素材缺失：语音静默降级（spec §13） */
      });
    return () => {
      cancelled = true;
      voiceRef.current?.dispose();
      voiceRef.current = null;
    };
  }, []);

  const showBubble = (text: string, ms: number) => {
    const gen = ++bubbleGen.current;
    setSubtitle(text);
    later(() => {
      if (bubbleGen.current === gen) setSubtitle(null);
    }, ms);
  };

  /** 播一组语音 + 动作 + 字幕（muted 只拦声音不拦动作/字幕，spec D5） */
  const playVoice = (group: VoiceGroup, action: PetAnimKey) => {
    playTransient(action, 1700);
    const player = voiceRef.current;
    if (!player) return;
    const entry = player.pick(group);
    if (!entry) return; // 空组静默跳过（spec E5）
    // 字幕独立于声音闸门：talkative 即显示，最短 2.5s（spec D5 + E4）；声音由 muted 单独拦截
    if (cfgRef.current.talkative) showBubble(entry.name, MIN_SPEECH_MS);
    player.play(entry, {
      muted: cfgRef.current.muted,
      onSubtitle: (name, ms) => {
        // 声音路径的字幕仅在非静音时生效，且时长与真实音频对齐（> 2.5s 时覆盖上面的兜底时长）
        if (!cfgRef.current.muted && cfgRef.current.talkative && ms > MIN_SPEECH_MS) {
          showBubble(name, ms);
        }
      },
    });
  };
  // 渲染期禁止写 ref（react-hooks/refs）：改在每次渲染后的 effect 中同步，供 Task 12 事件接线调用
  const playVoiceRef = useRef<(g: VoiceGroup, a: PetAnimKey) => void>(() => {});
  useEffect(() => {
    playVoiceRef.current = playVoice;
  });

  // ---- 状态卡片（Task 10；spec §5/C1-C4）----
  const [cards, setCards] = useState<PetCard[]>([]);
  const [moreCount, setMoreCount] = useState(0);
  const [candidates, setCandidates] = useState<JumpWindowCandidate[] | null>(null);
  const statusStateRef = useRef<PetStatusState | null>(null);
  const sessionIndexRef = useRef<Map<string, Session>>(new Map());
  const cardsWrapRef = useRef<HTMLDivElement | null>(null);
  const pendingAckRef = useRef(""); // 歧义跳转：点击卡片时先记待 ack 的会话 id（spec D12）
  useEffect(() => pet.registerInteractive(cardsWrapRef.current), [pet]);

  const { data } = useSessionsQuery();
  useEffect(() => {
    if (!data) return;
    sessionIndexRef.current = new Map(data.sessions.map((s) => [s.id, s]));
    const r = computePetStatus(data.sessions, statusStateRef.current, Date.now());
    statusStateRef.current = r.state;
    setCards(r.cards);
    setMoreCount(r.moreCount);
    // 事件接线在 Task 12 补充（newWaiting/newCompletion → 语音）
  }, [data]);

  /** 点击卡片跳转终端；歧义时弹候选浮层；失败静默保留卡片（spec C2/§13） */
  const jump = async (card: PetCard) => {
    const s = sessionIndexRef.current.get(card.id);
    if (!s) return;
    pendingAckRef.current = card.id; // 候选选中后按此 id ack
    try {
      const result = await invoke<{ type: string; windows?: JumpWindowCandidate[] }>("focus_session", {
        pid: s.pid, sessionId: s.id, agentType: s.agentType,
        projectName: s.projectName, lastMessage: s.lastMessage ?? undefined,
      });
      if (result.type === "ambiguous" && result.windows?.length) {
        setCandidates(result.windows); // 歧义候选浮层（spec D12）
        return;
      }
    } catch {
      return; // 跳转失败：卡片保留（spec §13）
    }
    ackDone(statusStateRef.current ?? {}, card.id); // 点击已读即消（spec C2）
    setCards(cardsFromState(statusStateRef.current ?? {}));
  };

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

  /** 使在途的 look 调度链失效（卸载清理用）：链式续期回调按代数自检后全部 no-op（Task 8 评审遗留） */
  const invalidateLookChain = () => {
    genRef.current.look += 1;
  };

  useEffect(() => {
    stepLoop();
    scheduleNextLook();
    return () => {
      cancelStep();
      stopLook();
      invalidateLookChain();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ---- 指针交互（spec A1/A2/A3/§8）----
  const lastDeltaRef = useRef({ dx: 0, dy: 0 });

  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return; // 右键留给菜单（spec A6）
    stopLook();
    if (!unlockedRef.current) {
      unlockedRef.current = true;
      voiceRef.current?.unlock(); // 手势内解锁自动播放（spec E6）
    }
    lastDeltaRef.current = { dx: 0, dy: 0 }; // 重置采样增量，避免上一次拖拽残留误判为「移动过」
    pet.beginDrag(e);
  };

  const onPointerMove = (e: React.PointerEvent) => {
    const r = pet.trackDrag(e);
    if (!r) return;
    void pet.moveBy(r.dx - lastDeltaRef.current.dx, r.dy - lastDeltaRef.current.dy);
    lastDeltaRef.current = { dx: r.dx, dy: r.dy };
    // 方向动画按 150ms 采样窗增量判定（原版逐帧增量语义，spec A3 阈值同原版）
    const dir: PetAnimKey | null =
      r.movedY < -8 ? "jumping" : r.movedX < -6 ? "run-left" : r.movedX > 6 ? "run-right" : null;
    const s = stateRef.current;
    if (dir) s.drag = dir;
    refreshAnim();
  };

  const onPointerUp = (_e: React.PointerEvent) => {
    const moved = lastDeltaRef.current.dx !== 0 || lastDeltaRef.current.dy !== 0;
    stateRef.current.drag = null;
    if (!moved) playTransient("waving", 1700); // 单击：固定挥手（spec A1）
    refreshAnim();
    pet.releaseDrag({
      gravity: cfgRef.current.gravity,
      onLand: () => {
        // 落地压扁回弹 + 补跳（spec §8）；transform 追加/移除 scaleY，基础 translateX(-50%) 不受影响
        const el = spriteRef.current;
        if (!el) return;
        el.style.transition = "transform 60ms ease-out";
        el.style.transform += " scaleY(0.55)";
        later(() => {
          el.style.transition = "transform 240ms cubic-bezier(.34,1.56,.64,1)";
          el.style.transform = el.style.transform.replace(" scaleY(0.55)", "");
          later(() => {
            el.style.transition = "";
            playTransient("jumping", 1500);
          }, 260);
        }, 60);
      },
    });
    lastDeltaRef.current = { dx: 0, dy: 0 };
  };

  const onDoubleClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    playVoice("general", cfgRef.current.dblAction); // 双击说话（spec A2）
  };

  // ---- 窗口几何同步（spec §4.2）：宽度恒 340×scale；高度 = 气泡区 + 精灵 + 间隙 + max(卡片区, 菜单) ----
  // Task 10 引入 cardsWrapRef、Task 12 引入 PetMenu（挂 menuWrapRef）后，把实测高度并入 Math.max；
  // menuWrapRef 本任务已测量（无菜单时高度为 0，与前向公式一致）
  const menuWrapRef = useRef<HTMLDivElement | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null); // Task 12 使用
  useLayoutEffect(() => {
    const baseH = px(50 + FRAME_H + 10);
    const menuH = menuWrapRef.current?.getBoundingClientRect().height ?? 0;
    const cardsH = cardsWrapRef.current?.getBoundingClientRect().height ?? 0;
    void pet.syncSize(px(340), Math.ceil(baseH + Math.max(cardsH, menuH)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cfg.scale, menu, cards, moreCount, candidates, pet.syncSize]);
  void setMenu; // Task 12 接线菜单开合时使用（先声明以满足 tsc noUnusedLocals）

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
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onDoubleClick={onDoubleClick}
      />
      {subtitle && (
        <div
          data-testid="pet-bubble"
          style={{
            position: "absolute",
            left: "50%",
            transform: "translateX(-50%)",
            bottom: 8,
            maxWidth: px(320),
            padding: `${px(6)}px ${px(12)}px`,
            fontSize: px(13),
            lineHeight: 1.4,
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
            borderRadius: px(12),
            pointerEvents: "none",
            background: "rgba(255,255,255,0.96)",
            color: "#7a4a2b",
            border: "1px solid rgba(122,74,43,0.35)",
            boxShadow: "0 2px 10px rgba(0,0,0,0.18)",
            zIndex: 2,
          }}
        >
          {subtitle}
        </div>
      )}
      {/* 状态卡片区（spec §5）：精灵上方居中，点击跳转终端 */}
      <div
        ref={cardsWrapRef}
        data-testid="pet-cards"
        style={{
          position: "absolute", bottom: px(50 + FRAME_H + 10), left: "50%",
          transform: "translateX(-50%)", display: "flex", flexDirection: "column",
          alignItems: "center", gap: px(5), width: px(320), zIndex: 3,
        }}
      >
        {cards.map((c) => (
          <div
            key={c.id}
            data-testid={`pet-card-${c.id}`}
            onClick={(e) => { e.stopPropagation(); void jump(c); }}
            style={{
              display: "flex", alignItems: "flex-start", gap: px(7), width: "100%",
              boxSizing: "border-box", padding: `${px(5)}px ${px(10)}px`, borderRadius: px(10),
              cursor: "pointer", fontSize: px(12), lineHeight: 1.45,
              background: "rgba(255,252,248,0.97)", border: "1px solid rgba(122,74,43,0.3)",
              boxShadow: "0 2px 8px rgba(0,0,0,0.14)",
            }}
          >
            <span style={{
              width: px(8), height: px(8), borderRadius: "50%", flex: "none", marginTop: px(4),
              background: c.light === "waiting" ? "#ef4444" : c.light === "running" ? "#eab308" : "#60a5fa",
              boxShadow: `0 0 0 2px ${c.light === "waiting" ? "rgba(239,68,68,.25)" : c.light === "running" ? "rgba(234,179,8,.25)" : "rgba(96,165,250,.25)"}`,
            }} />
            <div style={{ minWidth: 0 }}>
              <div style={{ fontWeight: 700, color: "#7a4a2b", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{c.title}</div>
              {c.lines.map((l, i) => (
                <div key={i} style={{ color: "#a07050", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{l}</div>
              ))}
            </div>
          </div>
        ))}
        {moreCount > 0 && (
          <div style={{ color: "#a07050", fontSize: px(11), background: "rgba(255,252,248,0.9)", borderRadius: 999, padding: `${px(2)}px ${px(8)}px` }}>
            +{moreCount} {t("pet.card.more")}
          </div>
        )}
      </div>
      {/* 歧义候选浮层（spec D12）：选中按 hwnd 聚焦并 ack 发起跳转的卡片 */}
      {candidates && (
        <div data-testid="pet-jump-candidates" style={{
          position: "absolute", bottom: px(50 + FRAME_H + 10), left: "50%", transform: "translateX(-50%)",
          width: px(320), maxHeight: px(240), overflowY: "auto", zIndex: 5,
          background: "rgba(30,30,34,0.96)", color: "#eee", borderRadius: px(10),
          fontSize: px(12), padding: `${px(4)}px 0`,
        }}>
          {candidates.map((w) => (
            <div key={w.hwnd} onClick={() => { void invoke("focus_hwnd", { hwnd: w.hwnd }); setCandidates(null); ackDone(statusStateRef.current ?? {}, pendingAckRef.current); setCards(cardsFromState(statusStateRef.current ?? {})); }}
              style={{ padding: `${px(3)}px ${px(14)}px`, cursor: "pointer" }}>
              {w.title}<span style={{ color: "#a1a1aa" }}> · {w.process}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
