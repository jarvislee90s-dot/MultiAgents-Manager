// PetMenu — 右键菜单：开关 / 大小三档 / 三场景动作绑定（带实时预览）/ 隐藏 / 关于（spec §9 B/D14/D15）
// D14：菜单只有三个动作绑定场景（双击/红灯/绿灯），errorAction 保留在配置但不进菜单
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
  anchor: { x: number; y: number };
  onClose(): void;
  onPreview(action: PetAction): void;
  onHide(): void;
}) {
  const { t } = useTranslation();
  const [cfg, setCfg] = useState<PetConfig>(() => loadConfig());
  const [page, setPage] = useState<MenuPage>(null);
  const ref = useRef<HTMLDivElement | null>(null);
  useEffect(() => subscribeConfig(() => setCfg(loadConfig())), []);

  // 菜单外点击 / Esc 关闭（spec B10）
  useEffect(() => {
    const onDown = (e: PointerEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) props.onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") props.onClose();
    };
    window.addEventListener("pointerdown", onDown, true);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onDown, true);
      window.removeEventListener("keydown", onKey);
    };
  }, [props]);

  // 动作子页实时预览：进入/切选项都触发，返回主菜单停（父组件以 onPreview(null) 停，Task 12 接线）
  useEffect(() => {
    if (page && page in ACTION_PAGE)
      props.onPreview(cfg[ACTION_PAGE[page as keyof typeof ACTION_PAGE]] as PetAction);
    else if (page === null) props.onPreview(null as unknown as PetAction);
  }, [page, cfg, props]);

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
        position: "fixed",
        left: props.anchor.x,
        top: props.anchor.y,
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
          {/* 开关行整行可点（spec B1 用例点击行文案即翻转）；按钮内 stopPropagation 防止双重切换 */}
          <div style={rowStyle} onClick={() => saveConfig({ muted: !cfg.muted })}>
            <span>{t("pet.menu.sound")}</span>
            {btn(
              !cfg.muted,
              () => saveConfig({ muted: !cfg.muted }),
              !cfg.muted ? t("pet.menu.on") : t("pet.menu.off")
            )}
          </div>
          <div style={rowStyle} onClick={() => saveConfig({ talkative: !cfg.talkative })}>
            <span>{t("pet.menu.subtitle")}</span>
            {btn(
              cfg.talkative,
              () => saveConfig({ talkative: !cfg.talkative }),
              cfg.talkative ? t("pet.menu.on") : t("pet.menu.off")
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
