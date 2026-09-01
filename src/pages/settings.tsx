import { useCallback, useEffect, useState } from "react";
import { emit } from "@tauri-apps/api/event";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { useTheme } from "@/components/common/theme-provider";
import { TitleBar } from "@/components/common/title-bar";
import { WindowFrame } from "@/components/common/window-frame";
import { LanguageToggle } from "@/components/common/language-toggle";
import { ShortcutInput } from "@/components/common/shortcut-input";
import { Moon, Sun, Monitor, Palette, Keyboard, Bell, Volume2 } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import {
  SOUND_IDS,
  getSoundConfig,
  saveSoundConfig,
  playSound,
  type SoundConfig,
} from "@/lib/audio";
import { registerShortcut, unregisterShortcut } from "@/lib/shortcut";
import { toggleWindow } from "@/lib/window";
import { toast } from "sonner";
import { Toaster } from "@/components/ui/sonner";
import { useAppTranslation } from "@/hooks/use-app-translation";

const SHORTCUT_KEY = "global-shortcut-show-main";

type SettingSection = "appearance" | "shortcut" | "notifications";

export default function SettingsPage() {
  const [shortcut, setShortcut] = useState<string>("");
  const [notificationsEnabled, setNotificationsEnabled] = useState(true);
  const [soundConfig, setSoundConfig] = useState<SoundConfig>(() => getSoundConfig());
  const [activeSection, setActiveSection] = useState<SettingSection>("appearance");
  const { t } = useAppTranslation();
  const { theme, setTheme } = useTheme();

  const handleShowMainWindow = useCallback(async () => {
    await toggleWindow("main");
  }, []);

  // 更新音效配置并持久化
  const updateSound = (patch: Partial<SoundConfig>) => {
    const next = { ...soundConfig, ...patch };
    setSoundConfig(next);
    saveSoundConfig(next);
  };

  useEffect(() => {
    // Load saved shortcut
    const savedShortcut = localStorage.getItem(SHORTCUT_KEY);
    if (savedShortcut) {
      setShortcut(savedShortcut);
      registerShortcut(savedShortcut, handleShowMainWindow);
    }
  }, [handleShowMainWindow]);

  useEffect(() => {
    const loadNotificationSetting = async () => {
      try {
        const value = await invoke<string | null>("get_setting", { key: "notifications_enabled" });
        setNotificationsEnabled(value !== "false");
      } catch {
        // 忽略错误
      }
    };
    loadNotificationSetting();
  }, []);

  const handleShortcutChange = async (newShortcut: string) => {
    const oldShortcut = shortcut;
    setShortcut(newShortcut);

    if (newShortcut) {
      localStorage.setItem(SHORTCUT_KEY, newShortcut);
      await registerShortcut(newShortcut, handleShowMainWindow, oldShortcut);
      // Notify main window to update shortcut
      await emit("shortcut-changed", { shortcut: newShortcut });
      toast.success(t("settings.shortcut.setSuccess", { shortcut: newShortcut }));
    } else {
      localStorage.removeItem(SHORTCUT_KEY);
      if (oldShortcut) {
        await unregisterShortcut(oldShortcut);
      }
      // Notify main window to clear shortcut
      await emit("shortcut-changed", { shortcut: "" });
      toast.info(t("settings.shortcut.cleared"));
    }
  };

  const toggleNotifications = async () => {
    const newValue = !notificationsEnabled;
    setNotificationsEnabled(newValue);
    await invoke("set_setting", { key: "notifications_enabled", value: String(newValue) });
    toast.success(
      newValue
        ? t("settings.notifications.enabledToast")
        : t("settings.notifications.disabledToast")
    );
  };

  const menuItems = [
    {
      id: "appearance" as SettingSection,
      label: t("settings.appearance.title"),
      icon: Palette,
    },
    {
      id: "shortcut" as SettingSection,
      label: t("settings.shortcut.title"),
      icon: Keyboard,
    },
    {
      id: "notifications" as SettingSection,
      label: t("settings.notifications.title"),
      icon: Bell,
    },
  ];

  return (
    <WindowFrame
      titleBar={<TitleBar title={t("settings.title")} showMaximize={false} />}
      contentClassName="flex flex-1 overflow-hidden"
    >
      <Toaster />
      <aside className="border-border flex w-40 flex-col border-r p-4">
        <nav className="flex-1 space-y-1">
          {menuItems.map((item) => {
            const Icon = item.icon;
            return (
              <button
                key={item.id}
                onClick={() => setActiveSection(item.id)}
                className={cn(
                  "flex w-full items-center gap-2 rounded-md px-3 py-2 text-sm transition-colors",
                  activeSection === item.id
                    ? "bg-accent text-accent-foreground font-medium"
                    : "text-muted-foreground hover:bg-accent/50 hover:text-foreground"
                )}
              >
                <Icon className="h-4 w-4" />
                {item.label}
              </button>
            );
          })}
        </nav>
      </aside>

      <div className="flex-1 overflow-auto">
        <div className="max-w-3xl p-4">
          {activeSection === "appearance" && (
            <div className="space-y-4">
              <div>
                <h2 className="mb-1 text-lg font-semibold">{t("settings.appearance.title")}</h2>
                <p className="text-muted-foreground text-sm">
                  {t("settings.appearance.description")}
                </p>
              </div>

              <div className="space-y-0">
                <div className="flex items-center justify-between py-2.5">
                  <label className="text-sm font-medium">{t("settings.appearance.theme")}</label>
                  <div className="flex gap-2">
                    <Button
                      variant={theme === "light" ? "default" : "outline"}
                      size="sm"
                      onClick={() => setTheme("light")}
                      className="flex items-center gap-1.5"
                    >
                      <Sun className="h-3.5 w-3.5" />
                      {t("settings.appearance.light")}
                    </Button>
                    <Button
                      variant={theme === "dark" ? "default" : "outline"}
                      size="sm"
                      onClick={() => setTheme("dark")}
                      className="flex items-center gap-1.5"
                    >
                      <Moon className="h-3.5 w-3.5" />
                      {t("settings.appearance.dark")}
                    </Button>
                    <Button
                      variant={theme === "system" ? "default" : "outline"}
                      size="sm"
                      onClick={() => setTheme("system")}
                      className="flex items-center gap-1.5"
                    >
                      <Monitor className="h-3.5 w-3.5" />
                      {t("settings.appearance.system")}
                    </Button>
                  </div>
                </div>

                <div className="border-t" />

                <div className="flex items-center justify-between py-2.5">
                  <label className="text-sm font-medium">{t("settings.appearance.language")}</label>
                  <LanguageToggle />
                </div>
              </div>
            </div>
          )}

          {activeSection === "shortcut" && (
            <div className="space-y-4">
              <div>
                <h2 className="mb-1 text-lg font-semibold">{t("settings.shortcut.title")}</h2>
                <p className="text-muted-foreground text-sm">
                  {t("settings.shortcut.description")}
                </p>
              </div>

              <div className="space-y-0">
                <div className="flex items-center justify-between py-2.5">
                  <div className="flex-1">
                    <label className="text-sm font-medium">{t("settings.shortcut.showMain")}</label>
                    <p className="text-muted-foreground mt-0.5 text-xs">
                      {t("settings.shortcut.showMainDesc")}
                    </p>
                  </div>
                  <ShortcutInput value={shortcut} onChange={handleShortcutChange} />
                </div>
              </div>
            </div>
          )}

          {activeSection === "notifications" && (
            <div className="space-y-4">
              <div>
                <h2 className="mb-1 text-lg font-semibold">
                  {t("settings.notifications.heading")}
                </h2>
                <p className="text-muted-foreground text-sm">
                  {t("settings.notifications.description")}
                </p>
              </div>
              <div className="space-y-0">
                <div className="flex items-center justify-between py-2.5">
                  <div className="flex-1">
                    <label className="text-sm font-medium">
                      {t("settings.notifications.desktop")}
                    </label>
                    <p className="text-muted-foreground mt-0.5 text-xs">
                      {t("settings.notifications.desktopDesc")}
                    </p>
                  </div>
                  <Button
                    variant={notificationsEnabled ? "default" : "outline"}
                    size="sm"
                    onClick={toggleNotifications}
                  >
                    {notificationsEnabled
                      ? t("settings.notifications.on")
                      : t("settings.notifications.off")}
                  </Button>
                </div>
                <div className="border-t" />
                <div className="space-y-3 py-2.5">
                  {/* 全局完成音：所有工具默认播放的音效 */}
                  <div className="flex items-center justify-between gap-2">
                    <label className="text-sm font-medium">
                      {t("settings.notifications.soundGlobalDefault")}
                    </label>
                    <div className="flex items-center gap-1.5">
                      <select
                        className="bg-background h-7 rounded border px-1.5 text-xs"
                        value={soundConfig.default}
                        onChange={(e) => updateSound({ default: e.currentTarget.value })}
                      >
                        {SOUND_IDS.map((id) => (
                          <option key={id} value={id}>
                            {id}
                          </option>
                        ))}
                        <option value="mute">{t("settings.notifications.soundMute")}</option>
                      </select>
                      {soundConfig.default !== "mute" && (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => playSound(soundConfig.default)}
                        >
                          <Volume2 className="mr-1 h-3 w-3" />
                          {t("settings.notifications.soundTest")}
                        </Button>
                      )}
                    </div>
                  </div>
                  {/* 工具专属音：覆盖全局默认，空值=跟随全局 */}
                  <label className="text-sm font-medium">
                    {t("settings.notifications.soundToolOverride")}
                  </label>
                  {(["claude", "codex", "opencode", "openclaw", "kimi"] as const).map((tool) => (
                    <div key={tool} className="flex items-center justify-between gap-2">
                      <span className="text-muted-foreground text-xs capitalize">{tool}</span>
                      <div className="flex items-center gap-1.5">
                        <select
                          className="bg-background h-7 rounded border px-1.5 text-xs"
                          value={soundConfig.tools[tool] ?? ""}
                          onChange={(e) =>
                            updateSound({
                              tools: {
                                ...soundConfig.tools,
                                [tool]: e.currentTarget.value || undefined,
                              },
                            })
                          }
                        >
                          <option value="">{t("settings.notifications.soundFollowGlobal")}</option>
                          {SOUND_IDS.map((id) => (
                            <option key={id} value={id}>
                              {id}
                            </option>
                          ))}
                          <option value="mute">{t("settings.notifications.soundMute")}</option>
                        </select>
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => {
                            // 试听：优先该工具当前生效音（未配置则回退全局）
                            const id = soundConfig.tools[tool] || soundConfig.default;
                            if (id !== "mute") playSound(id);
                          }}
                        >
                          <Volume2 className="mr-1 h-3 w-3" />
                          {t("settings.notifications.soundTest")}
                        </Button>
                      </div>
                    </div>
                  ))}
                </div>
                <div className="border-t" />
                <div className="flex items-center justify-between py-2.5">
                  <div className="flex-1">
                    <label className="text-sm font-medium">
                      {t("settings.notifications.floatTest")}
                    </label>
                    <p className="text-muted-foreground mt-0.5 text-xs">
                      {t("settings.notifications.floatTestDesc")}
                    </p>
                  </div>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={async () => {
                      try {
                        await invoke("show_notification_window", {
                          payload: {
                            agentType: "claude",
                            agentLabel: "Claude",
                            projectName: t("settings.notifications.testProject"),
                            statusColor: "yellow",
                            status: "waiting",
                            lastMessage: t("settings.notifications.testMessage"),
                            pid: 0,
                            sessionId: "test",
                          },
                        });
                      } catch (e) {
                        console.error("float preview failed:", e);
                      }
                    }}
                  >
                    <Bell className="mr-1.5 h-3.5 w-3.5" />
                    {t("settings.notifications.testFloat")}
                  </Button>
                </div>
              </div>
            </div>
          )}
        </div>
      </div>
    </WindowFrame>
  );
}
