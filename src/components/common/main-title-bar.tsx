import { Moon, Sun, Info, Settings } from "lucide-react";
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTheme } from "@/components/common/theme-provider";
import { createWindow } from "@/lib/window";
import { TitleBar } from "@/components/common/title-bar";
import { LanguageToggle } from "@/components/common/language-toggle";
import { useTranslation } from "react-i18next";
import { loadVisible, saveVisible, subscribeConfig } from "@/components/pet/petConfig";
import packageJson from "../../../package.json";

export function MainTitleBar() {
  const { theme, setTheme } = useTheme();
  const { t } = useTranslation();
  // 桌宠开关状态：与 petConfig 双向同步（设置页/宠物菜单/托盘改动经订阅回流）
  const [petOn, setPetOn] = useState(() => loadVisible());
  useEffect(
    () =>
      subscribeConfig(() => {
        setPetOn(loadVisible());
      }),
    []
  );

  const handleTogglePet = async () => {
    const next = !petOn;
    saveVisible(next);
    setPetOn(next);
    try {
      await invoke("set_pet_visible", { visible: next });
    } catch (e) {
      console.error("set_pet_visible failed:", e);
    }
  };

  const handleToggleTheme = () => {
    setTheme(theme === "dark" ? "light" : "dark");
  };

  const handleOpenAbout = async () => {
    await createWindow("about", {
      title: t("about.title"),
      url: "/about",
      width: 500,
      height: 400,
      resizable: false,
      maximizable: false,
      minimizable: false,
      decorations: false,
      transparent: true,
      shadow: false,
      alwaysOnTop: true,
      parent: "main",
    });
  };

  const handleOpenSettings = async () => {
    await createWindow("settings", {
      title: t("settings.title"),
      url: "/settings",
      width: 600,
      height: 500,
      resizable: true,
      maximizable: true,
      minimizable: false,
      decorations: false,
      transparent: true,
      shadow: false,
      parent: "main",
    });
  };

  return (
    <TitleBar
      title={`${t("app.title")} v${packageJson.version}`}
      rightActions={
        <>
          <button
            onClick={handleOpenSettings}
            className="title-bar-btn mr-1"
            aria-label={t("settings.button")}
            tabIndex={-1}
          >
            <Settings className="h-4 w-4" />
          </button>

          <button
            onClick={handleTogglePet}
            className="title-bar-btn mr-1 text-base leading-none"
            aria-label={t("home.petToggle")}
            title={t("home.petToggle")}
            tabIndex={-1}
          >
            <span className={petOn ? "" : "opacity-45 grayscale"}>🦊</span>
          </button>

          <button
            onClick={handleOpenAbout}
            className="title-bar-btn mr-1"
            aria-label={t("about.button")}
            tabIndex={-1}
          >
            <Info className="h-4 w-4" />
          </button>

          <LanguageToggle />

          <button
            onClick={handleToggleTheme}
            className="title-bar-btn mr-0.5"
            aria-label={t("theme.toggle")}
            tabIndex={-1}
          >
            {theme === "dark" ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
          </button>
        </>
      }
    />
  );
}
