import { useCallback, useEffect, useRef, useState } from "react";
import { emit } from "@tauri-apps/api/event";
import { useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
import { useTheme } from "@/components/common/theme-provider";
import { TitleBar } from "@/components/common/title-bar";
import { WindowFrame } from "@/components/common/window-frame";
import { LanguageToggle } from "@/components/common/language-toggle";
import { ShortcutInput } from "@/components/common/shortcut-input";
import { Moon, Sun, Monitor, Palette, Keyboard, Bell, Volume2, Dog, Wrench } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { Switch } from "@/components/ui/switch";
import {
  loadVisible,
  saveVisible,
  loadConfig,
  saveConfig,
  subscribeConfig,
  PET_SCALES,
  type PetConfig,
} from "@/components/pet/petConfig";
import {
  SOUND_IDS,
  getSoundConfig,
  saveSoundConfig,
  playSound,
  type SoundConfig,
} from "@/lib/audio";
import { registerShortcut, unregisterShortcut } from "@/lib/shortcut";
import { toggleWindow } from "@/lib/window";
import { useEnabledToolsQuery } from "@/lib/query/queries/tools";
import { toast } from "sonner";
import { Toaster } from "@/components/ui/sonner";
import { useAppTranslation } from "@/hooks/use-app-translation";

const SHORTCUT_KEY = "global-shortcut-show-main";

type SettingSection = "appearance" | "shortcut" | "notifications" | "pet" | "tools";

// 工具管理行（后端 ToolSetting，serde camelCase）
type ToolRow = {
  toolId: string;
  name: string;
  enabled: boolean;
  installed: boolean;
  managed: boolean;
};

export default function SettingsPage() {
  const [shortcut, setShortcut] = useState<string>("");
  const [notificationsEnabled, setNotificationsEnabled] = useState(true);
  const [soundConfig, setSoundConfig] = useState<SoundConfig>(() => getSoundConfig());
  const [activeSection, setActiveSection] = useState<SettingSection>("appearance");
  // 桌宠状态：复用 petConfig（localStorage 单后端），跨窗口改动经 subscribeConfig 回流
  const [petVisible, setPetVisible] = useState(() => loadVisible());
  const [petCfg, setPetCfg] = useState(() => loadConfig());
  // 工具管理状态（spec W5）：本地草稿 + 脏标记，保存时统一批量应用
  const [toolRows, setToolRows] = useState<ToolRow[]>([]);
  const [toolDirty, setToolDirty] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  // 未保存离开拦截：缓存「放弃更改并跳转」的回调
  const [leaveGuard, setLeaveGuard] = useState<null | (() => void)>(null);
  // 加载成功后的 enabled 快照（state 而非 ref：changedRows 渲染期 diff 需响应式读取）
  const [savedToolEnabled, setSavedToolEnabled] = useState<Record<string, boolean>>({});
  // 离开拦截选「保存」时缓存跳转，应用成功后执行
  const pendingJumpRef = useRef<(() => void) | null>(null);
  const { t } = useAppTranslation();
  const { theme, setTheme } = useTheme();
  const queryClient = useQueryClient();
  // 启用工具列表（后端下发，勾选状态驱动；声音覆盖行随勾选增减）
  const { data: enabledTools = [] } = useEnabledToolsQuery();

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

  useEffect(
    () =>
      subscribeConfig(() => {
        setPetVisible(loadVisible());
        setPetCfg(loadConfig());
      }),
    []
  );

  const loadToolSettings = useCallback(async () => {
    try {
      const rows = await invoke<ToolRow[]>("get_tool_settings");
      setToolRows(rows);
      // 记录快照，供 changedRows diff 与保存后复位
      setSavedToolEnabled(Object.fromEntries(rows.map((r) => [r.toolId, r.enabled])));
      setToolDirty(false);
    } catch (e) {
      console.error("get_tool_settings failed:", e);
    }
  }, []);

  // 进入工具分区时拉取勾选状态
  useEffect(() => {
    if (activeSection === "tools") void loadToolSettings();
  }, [activeSection, loadToolSettings]);

  // 脏标记时拦截窗口关闭（标签页/窗口 beforeunload 场景）
  useEffect(() => {
    if (!toolDirty) return;
    const handler = (e: BeforeUnloadEvent) => {
      e.preventDefault();
      e.returnValue = "";
    };
    window.addEventListener("beforeunload", handler);
    return () => window.removeEventListener("beforeunload", handler);
  }, [toolDirty]);

  // 开关点击仅改本地草稿（不立即生效），保存时批量应用
  const toggleTool = (toolId: string, next: boolean) => {
    setToolRows((rows) => rows.map((r) => (r.toolId === toolId ? { ...r, enabled: next } : r)));
    setToolDirty(true);
  };

  // 与快照 diff 出本次变更行
  const changedRows = toolRows.filter(
    (r) => r.enabled !== (savedToolEnabled[r.toolId] ?? r.enabled)
  );

  // 分区切换守卫：工具分区有未保存更改时先弹三选拦截
  const switchSection = (next: SettingSection) => {
    if (next === activeSection) return;
    if (activeSection === "tools" && toolDirty) {
      setLeaveGuard(() => () => {
        setToolDirty(false);
        setActiveSection(next);
      });
      return;
    }
    setActiveSection(next);
  };

  // 批量应用变更；成功后复位草稿、失效缓存并执行缓存跳转
  const applyChanges = async () => {
    try {
      const result = await invoke<{
        restored: string[];
        restoredMcps: string[];
        rebuildFailed: string[];
      }>("update_tool_settings", {
        changes: changedRows.map((r) => ({ toolId: r.toolId, enabled: r.enabled })),
      });
      toast.success(t("settings.tools.applied"));
      if (result.rebuildFailed.length) {
        toast.warning(`rebuild failed: ${result.rebuildFailed.join(", ")}`);
      }
      setConfirmOpen(false);
      await loadToolSettings();
      // 全量失效 react-query 缓存：看板 / 资源分布立即刷新
      await queryClient.invalidateQueries();
      const jump = pendingJumpRef.current;
      pendingJumpRef.current = null;
      jump?.();
    } catch (e) {
      toast.error(String(e));
    }
  };

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

  // 桌宠显隐：写 localStorage + 同步 Rust 端窗口创建/销毁
  const onPetVisibleChange = async (v: boolean) => {
    saveVisible(v);
    setPetVisible(v);
    try {
      await invoke("set_pet_visible", { visible: v });
    } catch (e) {
      console.error("set_pet_visible failed:", e);
    }
    toast.success(v ? t("settings.pet.enabledToast") : t("settings.pet.disabledToast"));
  };

  // 桌宠配置：仅置顶变化需要同步 Rust 端 always-on-top
  const onPetCfgChange = (patch: Partial<PetConfig>) => {
    saveConfig(patch);
    setPetCfg(loadConfig());
    if (patch.alwaysOnTop !== undefined) {
      invoke("set_pet_always_on_top", { onTop: patch.alwaysOnTop }).catch(() => {});
    }
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
    {
      id: "pet" as SettingSection,
      label: t("settings.pet.title"),
      icon: Dog,
    },
    {
      id: "tools" as SettingSection,
      label: t("settings.tools.title"),
      icon: Wrench,
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
                onClick={() => switchSection(item.id)}
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
                  {enabledTools.map((tool) => {
                    // 后端工具 id 与 audio.ts 的 SoundConfig.tools 键一致
                    const key = tool.id as keyof SoundConfig["tools"];
                    return (
                      <div key={key} className="flex items-center justify-between gap-2">
                        <span className="text-muted-foreground text-xs">{tool.label}</span>
                        <div className="flex items-center gap-1.5">
                          <select
                            className="bg-background h-7 rounded border px-1.5 text-xs"
                            value={soundConfig.tools[key] ?? ""}
                            onChange={(e) =>
                              updateSound({
                                tools: {
                                  ...soundConfig.tools,
                                  [key]: e.currentTarget.value || undefined,
                                },
                              })
                            }
                          >
                            <option value="">
                              {t("settings.notifications.soundFollowGlobal")}
                            </option>
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
                              const id = soundConfig.tools[key] || soundConfig.default;
                              if (id !== "mute") playSound(id);
                            }}
                          >
                            <Volume2 className="mr-1 h-3 w-3" />
                            {t("settings.notifications.soundTest")}
                          </Button>
                        </div>
                      </div>
                    );
                  })}
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

          {activeSection === "pet" && (
            <div className="space-y-4">
              <div>
                <h2 className="mb-1 text-lg font-semibold">{t("settings.pet.title")}</h2>
                <p className="text-muted-foreground text-sm">{t("settings.pet.desc")}</p>
              </div>
              <div className="space-y-0">
                {/* 开启开关：显隐同步 Rust 端创建/销毁宠物窗口 */}
                <div className="flex items-center justify-between py-2.5">
                  <div className="flex-1">
                    <label className="text-sm font-medium">{t("settings.pet.enable")}</label>
                  </div>
                  <Switch checked={petVisible} onCheckedChange={onPetVisibleChange} />
                </div>
                <div className="border-t" />
                {/* 置顶开关：置顶时抑制主窗口浮窗通知（spec D4） */}
                <div className="flex items-center justify-between py-2.5">
                  <div className="flex-1">
                    <label className="text-sm font-medium">{t("settings.pet.alwaysOnTop")}</label>
                  </div>
                  <Switch
                    checked={petCfg.alwaysOnTop}
                    onCheckedChange={(v) => onPetCfgChange({ alwaysOnTop: v })}
                  />
                </div>
                <div className="border-t" />
                {/* 大小三档 */}
                <div className="flex items-center justify-between py-2.5">
                  <label className="text-sm font-medium">{t("settings.pet.scale")}</label>
                  <div className="flex gap-1">
                    {PET_SCALES.map((s) => (
                      <button
                        key={s}
                        onClick={() => onPetCfgChange({ scale: s })}
                        className={`rounded px-3 py-1 text-sm transition-colors ${
                          petCfg.scale === s
                            ? "bg-accent text-accent-foreground font-medium"
                            : "text-muted-foreground hover:bg-accent/50"
                        }`}
                      >
                        {s === 0.75
                          ? t("pet.scale.small")
                          : s === 1
                            ? t("pet.scale.medium")
                            : t("pet.scale.large")}
                      </button>
                    ))}
                  </div>
                </div>
              </div>
            </div>
          )}

          {activeSection === "tools" && (
            <div className="space-y-4">
              <div>
                <h2 className="mb-1 text-lg font-semibold">{t("settings.tools.title")}</h2>
                <p className="text-muted-foreground text-sm">{t("settings.tools.hint")}</p>
              </div>
              {/* 行式开关列表：名称 + 安装状态 badge + Switch */}
              <div className="divide-border divide-y rounded-md border">
                {toolRows.map((r) => (
                  <div key={r.toolId} className="flex items-center justify-between px-3 py-2">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium">{r.name}</span>
                      <span
                        className={cn(
                          "rounded px-1.5 py-0.5 text-[10px]",
                          r.installed
                            ? "bg-emerald-500/10 text-emerald-500"
                            : "bg-muted text-muted-foreground"
                        )}
                      >
                        {r.installed
                          ? t("settings.tools.installed")
                          : t("settings.tools.notInstalled")}
                      </span>
                    </div>
                    <Switch checked={r.enabled} onCheckedChange={(v) => toggleTool(r.toolId, v)} />
                  </div>
                ))}
              </div>
              {toolDirty && (
                <Button onClick={() => setConfirmOpen(true)}>{t("settings.tools.save")}</Button>
              )}
            </div>
          )}
        </div>
      </div>

      {/* 保存确认弹窗：列出变更行并按变更方向提示影响 */}
      <Dialog
        open={confirmOpen}
        onOpenChange={(v) => {
          setConfirmOpen(v);
          // 取消确认时丢弃缓存的跳转，留在本页
          if (!v) pendingJumpRef.current = null;
        }}
      >
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>{t("settings.tools.confirmTitle")}</DialogTitle>
            <DialogDescription>{t("settings.tools.confirmDesc")}</DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            {changedRows.map((r) => (
              <div key={r.toolId} className="flex items-start justify-between gap-3 text-sm">
                <span className="font-medium">{r.name}</span>
                <span
                  className={cn(
                    "text-right text-xs",
                    r.enabled
                      ? "text-emerald-500"
                      : r.managed
                        ? "text-amber-500"
                        : "text-muted-foreground"
                  )}
                >
                  {r.enabled
                    ? t("settings.tools.enableItem")
                    : r.managed
                      ? t("settings.tools.restoreItem")
                      : t("settings.tools.disableItem")}
                </span>
              </div>
            ))}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setConfirmOpen(false)}>
              {t("settings.tools.cancel")}
            </Button>
            <Button onClick={() => void applyChanges()}>{t("settings.tools.confirm")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* 未保存离开拦截：保存 / 放弃更改 / 继续编辑 三选 */}
      <Dialog open={leaveGuard !== null} onOpenChange={(v) => !v && setLeaveGuard(null)}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>{t("settings.tools.unsavedTitle")}</DialogTitle>
            <DialogDescription>{t("settings.tools.unsavedDesc")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setLeaveGuard(null)}>
              {t("settings.tools.keepEditing")}
            </Button>
            <Button
              variant="outline"
              onClick={() => {
                // 放弃更改：缓存回调内含「清脏 + 跳转」
                const jump = leaveGuard;
                setLeaveGuard(null);
                jump?.();
              }}
            >
              {t("settings.tools.discard")}
            </Button>
            <Button
              onClick={() => {
                // 保存：记下跳转，确认应用成功后再执行
                pendingJumpRef.current = leaveGuard;
                setLeaveGuard(null);
                setConfirmOpen(true);
              }}
            >
              {t("settings.tools.save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </WindowFrame>
  );
}
