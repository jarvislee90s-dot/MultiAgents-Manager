// PetMenu — 右键菜单：开关 / 大小三档 / 三场景动作绑定（带实时预览）/ 隐藏 / 关于（spec §9 B/D14/D15）
// D14：菜单只有三个动作绑定场景（双击/红灯/绿灯），errorAction 保留在配置但不进菜单
// Fix 1：根节点改为 in-flow（static）——外层包裹 div（FoxbellPet menuWrapRef）absolute 定位并
// 以 BOTTOM 对齐光标向上展开；本组件只负责内容高度，定位职责全部上移到包裹层。
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  loadConfig,
  saveConfig,
  subscribeConfig,
  PET_ACTIONS,
  PET_SCALES,
  type PetAction,
  type PetConfig,
  type PetScale,
} from "./petConfig";

type MenuPage = null | "Size" | "Dbl" | "Red" | "Green" | "About";
const ACTION_PAGE: Record<"Dbl" | "Red" | "Green", keyof PetConfig> = {
  Dbl: "dblAction",
  Red: "approvalAction",
  Green: "doneAction",
};

export function PetMenu(props: {
  onClose(): void;
  onPreview(action: PetAction | null): void;
  onHide(): void;
  voiceCapable: boolean;
  subtitleCapable: boolean;
}) {
  const { t } = useTranslation();
  const [cfg, setCfg] = useState<PetConfig>(() => loadConfig());
  const [page, setPage] = useState<MenuPage>(null);
  const ref = useRef<HTMLDivElement | null>(null);
  useEffect(() => subscribeConfig(() => setCfg(loadConfig())), []);

  // 菜单外点击 / Esc 关闭（spec B10）。pointerdown 捕获阶段监听：点精灵（菜单外）同样命中
  // contains 判定 → 关闭；精灵自身的 onContextMenu 可在随后重新打开
  const { onClose, onPreview } = props;
  useEffect(() => {
    const onDown = (e: PointerEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("pointerdown", onDown, true);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onDown, true);
      window.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  // 动作子页实时预览：进入/切选项都触发，返回主菜单停（父组件以 onPreview(null) 停，Task 12 接线）
  // Fix 3：依赖收窄到 [page, cfg, onPreview]——配合父组件的 useCallback 稳定身份，
  // 消除「每次渲染新 props 对象 → effect 重触发 → stop/setInterval 抖动」的循环
  useEffect(() => {
    if (page && page in ACTION_PAGE)
      onPreview(cfg[ACTION_PAGE[page as keyof typeof ACTION_PAGE]] as PetAction);
    else if (page === null) onPreview(null);
  }, [page, cfg, onPreview]);

  const rowStyle: React.CSSProperties = {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 10,
    padding: "3px 14px",
    cursor: "pointer",
  };
  const itemStyle: React.CSSProperties = { padding: "3px 14px", cursor: "pointer" };
  const btn = (on: boolean, onClick: () => void, label: string) => (
    <button
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      style={{
        background: on ? "#16a34a" : "#3f3f46",
        color: "#eee",
        border: "none",
        borderRadius: 6,
        fontSize: 12,
        padding: "1px 10px",
        cursor: "pointer",
      }}
    >
      {label}
    </button>
  );

  const actionLabel = (a: PetAction) => t(`pet.action.${a}`);
  const scaleLabel = (s: PetScale) =>
    s === 0.75 ? t("pet.scale.small") : s === 1 ? t("pet.scale.medium") : t("pet.scale.large");

  return (
    <div
      ref={ref}
      data-testid="pet-menu"
      style={{
        minWidth: 170,
        background: "rgba(30,30,34,0.96)",
        color: "#eee",
        fontSize: 13,
        lineHeight: 1.9,
        borderRadius: 10,
        padding: "4px 0",
        boxShadow: "0 6px 20px rgba(0,0,0,0.4)",
        zIndex: 10,
      }}
    >
      {page === "About" ? (
        <div data-testid="pet-menu-about" style={itemStyle} onClick={props.onClose}>
          {t("pet.menu.aboutText")}
        </div>
      ) : page === "Size" ? (
        <>
          <div style={{ ...itemStyle, cursor: "pointer" }} onClick={() => setPage(null)}>
            {t("pet.menu.back")}
          </div>
          {PET_SCALES.map((s: PetScale) => (
            <div
              key={s}
              style={{ ...itemStyle, color: cfg.scale === s ? "#fbbf24" : undefined }}
              onClick={() => {
                saveConfig({ scale: s });
                setPage(null);
              }}
            >
              {scaleLabel(s)}
            </div>
          ))}
        </>
      ) : page && page in ACTION_PAGE ? (
        <>
          <div style={itemStyle} onClick={() => setPage(null)}>
            {t("pet.menu.back")}
          </div>
          {PET_ACTIONS.map((a) => (
            <div
              key={a}
              style={{
                ...itemStyle,
                color:
                  cfg[ACTION_PAGE[page as keyof typeof ACTION_PAGE]] === a ? "#fbbf24" : undefined,
              }}
              onClick={() => {
                saveConfig({
                  [ACTION_PAGE[page as keyof typeof ACTION_PAGE]]: a,
                } as Partial<PetConfig>);
                setPage(null);
              }}
            >
              {actionLabel(a)}
            </div>
          ))}
        </>
      ) : (
        <>
          {/* 开关行整行可点（spec B1 用例点击行文案即翻转）；按钮内 stopPropagation 防止双重切换。
              能力门控（spec §5.2）：不具备语音/字幕能力时置灰不可切换 + tooltip 说明原因（EP9） */}
          <div
            data-testid="pet-menu-row-sound"
            title={!props.voiceCapable ? t("pet.menu.soundNoCap") : undefined}
            style={{
              ...rowStyle,
              opacity: props.voiceCapable ? 1 : 0.5,
              cursor: props.voiceCapable ? "pointer" : "not-allowed",
            }}
            onClick={() => {
              if (props.voiceCapable) saveConfig({ muted: !cfg.muted });
            }}
          >
            <span>{t("pet.menu.sound")}</span>
            {btn(
              !cfg.muted,
              () => {
                if (props.voiceCapable) saveConfig({ muted: !cfg.muted });
              },
              !cfg.muted && props.voiceCapable ? t("pet.menu.on") : t("pet.menu.off")
            )}
          </div>
          <div
            data-testid="pet-menu-row-subtitle"
            title={!props.subtitleCapable ? t("pet.menu.subtitleNoCap") : undefined}
            style={{
              ...rowStyle,
              opacity: props.subtitleCapable ? 1 : 0.5,
              cursor: props.subtitleCapable ? "pointer" : "not-allowed",
            }}
            onClick={() => {
              if (props.subtitleCapable) saveConfig({ talkative: !cfg.talkative });
            }}
          >
            <span>{t("pet.menu.subtitle")}</span>
            {btn(
              cfg.talkative,
              () => {
                if (props.subtitleCapable) saveConfig({ talkative: !cfg.talkative });
              },
              cfg.talkative && props.subtitleCapable ? t("pet.menu.on") : t("pet.menu.off")
            )}
          </div>
          <div style={rowStyle} onClick={() => saveConfig({ gravity: !cfg.gravity })}>
            <span>{t("pet.menu.physics")}</span>
            {btn(
              cfg.gravity,
              () => saveConfig({ gravity: !cfg.gravity }),
              cfg.gravity ? t("pet.menu.on") : t("pet.menu.off")
            )}
          </div>
          <div style={rowStyle} onClick={() => saveConfig({ alwaysOnTop: !cfg.alwaysOnTop })}>
            <span>{t("pet.menu.onTop")}</span>
            {btn(
              cfg.alwaysOnTop,
              () => saveConfig({ alwaysOnTop: !cfg.alwaysOnTop }),
              cfg.alwaysOnTop ? t("pet.menu.on") : t("pet.menu.off")
            )}
          </div>
          <div style={{ height: 1, margin: "4px 10px", background: "rgba(255,255,255,0.12)" }} />
          <div style={rowStyle} onClick={() => setPage("Size")}>
            <span>{t("pet.menu.size")}</span>
            <span style={{ color: "#a1a1aa", fontSize: 12 }}>{scaleLabel(cfg.scale)}</span>
          </div>
          {(["Dbl", "Red", "Green"] as const).map((p) => (
            <div key={p} style={rowStyle} onClick={() => setPage(p)}>
              <span>
                {t(
                  `pet.menu.${p === "Dbl" ? "dblAction" : p === "Red" ? "redAction" : "greenAction"}`
                )}
              </span>
              <span style={{ color: "#a1a1aa", fontSize: 12 }}>
                {actionLabel(cfg[ACTION_PAGE[p]] as PetAction)}
              </span>
            </div>
          ))}
          <div style={{ height: 1, margin: "4px 10px", background: "rgba(255,255,255,0.12)" }} />
          <div style={itemStyle} onClick={props.onHide}>
            {t("pet.menu.hide")}
          </div>
          <div style={itemStyle} onClick={() => setPage("About")}>
            {t("pet.menu.about")}
          </div>
        </>
      )}
    </div>
  );
}
