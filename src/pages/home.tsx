import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { WindowFrame } from "@/components/common/window-frame";
import { MainTitleBar } from "@/components/common/main-title-bar";
import { UpdaterDialog } from "@/components/common/updater-dialog";
import { Toaster } from "@/components/ui/sonner";
import { SessionGrid } from "@/components/sessions/SessionGrid";
import { ExtensionList } from "@/components/resources/ExtensionList";
import { Monitor, Package } from "lucide-react";
import { useSessions } from "@/hooks/useSessions";
import { useNotification } from "@/hooks/useNotification";
import { useSessionStore } from "@/stores/sessionStore";
import { registerShortcut } from "@/lib/shortcut";
import { toggleWindow } from "@/lib/window";
import { loadVisible, subscribeConfig } from "@/components/pet/petConfig";
import { PetStartupGuard } from "@/components/pet/PetStartupGuard";
import { useAppTranslation } from "@/hooks/use-app-translation";
import { Activity, AlertCircle } from "lucide-react";
import { NotificationBell } from "@/components/notifications/NotificationBell";

const SHORTCUT_KEY = "global-shortcut-show-main";

export default function HomePage() {
  useSessions();
  useNotification();
  const [activeTab, setActiveTab] = useState<"dashboard" | "extensions">("dashboard");
  const { sessions, totalCount, waitingCount, loading } = useSessionStore();
  const { t } = useAppTranslation();

  // 桌宠开关状态：与 petConfig 双向同步（标题栏开关/设置页/托盘改动经订阅回流）；
  // 开关本体在 MainTitleBar，此处状态仅供托盘菜单文案
  const [petOn, setPetOn] = useState(() => loadVisible());
  useEffect(() => subscribeConfig(() => setPetOn(loadVisible())), []);

  useEffect(() => {
    const unlistenShortcutChanged = listen<{ shortcut: string }>(
      "shortcut-changed",
      async (event) => {
        const newShortcut = event.payload.shortcut;
        if (newShortcut) {
          await registerShortcut(newShortcut, async () => {
            await toggleWindow("main");
          });
        }
      }
    );

    const initTrayMenu = async () => {
      try {
        await invoke("update_tray_menu", {
          showText: t("tray.show"),
          quitText: t("tray.quit"),
          petText: petOn ? t("tray.petHide") : t("tray.petShow"),
        });
      } catch (error) {
        console.error("Failed to initialize tray menu:", error);
      }
    };
    initTrayMenu();

    const initShortcut = async () => {
      const savedShortcut = localStorage.getItem(SHORTCUT_KEY);
      if (savedShortcut) {
        await registerShortcut(savedShortcut, async () => {
          await toggleWindow("main");
        });
      }
    };
    initShortcut();

    return () => {
      unlistenShortcutChanged.then((fn) => fn());
    };
    // petOn 变化时重跑本 effect，托盘桌宠文案随开关/语言刷新
  }, [t, petOn]);

  return (
    <WindowFrame
      titleBar={<MainTitleBar />}
      contentClassName="container mx-auto flex flex-1 flex-col gap-3 overflow-hidden p-4"
    >
      <UpdaterDialog />
      <Toaster />

      {/* 状态摘要栏 */}
      <div className="flex items-center justify-between gap-4">
        <div className="flex items-center gap-4">
          <div className="flex items-center gap-2">
            <Activity className="text-muted-foreground h-4 w-4" />
            <span className="text-sm font-semibold">{totalCount}</span>
            <span className="text-muted-foreground text-xs">{t("home.sessions")}</span>
          </div>
          {waitingCount > 0 && (
            <div className="flex items-center gap-2">
              <AlertCircle className="h-4 w-4 text-red-500" />
              <span className="text-sm font-semibold text-red-500">{waitingCount}</span>
              <span className="text-muted-foreground text-xs">{t("home.waiting")}</span>
            </div>
          )}
        </div>
        {loading && <span className="text-muted-foreground text-xs">{t("home.loading")}</span>}
      </div>

      {/* 标签栏 */}
      <div className="flex items-center justify-between border-b pb-1">
        <div className="flex gap-1">
          <button
            onClick={() => setActiveTab("dashboard")}
            className={`flex items-center gap-1.5 rounded px-3 py-1 text-sm transition-colors ${
              activeTab === "dashboard"
                ? "bg-accent text-accent-foreground font-medium"
                : "text-muted-foreground hover:bg-accent/50"
            }`}
          >
            <Monitor className="h-3.5 w-3.5" />
            {t("home.tabDashboard")}
          </button>
          <button
            onClick={() => setActiveTab("extensions")}
            className={`flex items-center gap-1.5 rounded px-3 py-1 text-sm transition-colors ${
              activeTab === "extensions"
                ? "bg-accent text-accent-foreground font-medium"
                : "text-muted-foreground hover:bg-accent/50"
            }`}
          >
            <Package className="h-3.5 w-3.5" />
            {t("home.tabResources")}
          </button>
        </div>
        {/* 桌宠开关已上移至标题栏（MainTitleBar 齿轮右侧）；此处不再保留副本 */}
        <NotificationBell />
      </div>

      {/* 内容区 */}
      <div className="flex-1 overflow-y-auto">
        {activeTab === "dashboard" ? <SessionGrid sessions={sessions} /> : <ExtensionList />}
      </div>
      {/* 启动校验弹窗（EP2）：外部宠物素材异常时主窗口确认，宠物窗口先行降级 */}
      <PetStartupGuard />
    </WindowFrame>
  );
}
