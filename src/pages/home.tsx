import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { WindowFrame } from "@/components/common/window-frame";
import { MainTitleBar } from "@/components/common/main-title-bar";
import { UpdaterDialog } from "@/components/common/updater-dialog";
import { Toaster } from "@/components/ui/sonner";
import { SessionGrid } from "@/components/sessions/SessionGrid";
import { ExtensionList } from "@/components/resources/ExtensionList";
import { CheckCheck, Monitor, Package } from "lucide-react";
import { useQueryClient } from "@tanstack/react-query";
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
import { useLegacySkillMigration } from "@/hooks/useLegacySkillMigration";
import { LegacySkillMigrationDialog } from "@/components/resources/LegacySkillMigrationDialog";

const SHORTCUT_KEY = "global-shortcut-show-main";

export default function HomePage() {
  useSessions();
  useNotification();
  // 遗留 codex 技能链接检测（spec §4.3）：mount 即检测，命中 ≥1 条时弹一次性迁移对话框
  const legacyMigration = useLegacySkillMigration();
  const [activeTab, setActiveTab] = useState<"dashboard" | "extensions">("dashboard");
  const { sessions, totalCount, waitingCount, loading } = useSessionStore();
  const { t } = useAppTranslation();
  const queryClient = useQueryClient();

  // 一键清除已完成卡片（绿/空闲）：对每张绿卡复用单卡 X 的同款语义——
  // 未读卡标已读（mark_session_read），其余 dismiss（dismiss_session_card）；
  // 全部完成后立即失效轮询缓存，不等 3 秒轮询自然生效
  const finishedCards = sessions.filter((s) => s.status === "idle" || s.status === "finished");
  const clearFinishedCards = async () => {
    await Promise.allSettled(
      finishedCards.map((s) =>
        s.unread
          ? invoke("mark_session_read", { agentType: s.agentType, sessionId: s.id })
          : invoke("dismiss_session_card", {
              agentType: s.agentType,
              sessionId: s.id,
              status: s.status,
            })
      )
    );
    queryClient.invalidateQueries({ queryKey: ["sessions"] });
  };

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
        // Task 16 统一重建：基础项 + 预设项一次成型（原 update_tray_menu 会丢预设项）
        await invoke("refresh_tray", {
          presetsLabel: t("tray.presetsLabel"),
          showText: t("tray.show"),
          petText: petOn ? t("tray.petHide") : t("tray.petShow"),
          quitText: t("tray.quit"),
          remoteOnText: t("tray.remote"), // M4 T4：远程开关项标签（勾选态 Rust 侧自查）
        });
      } catch (error) {
        console.error("Failed to initialize tray menu:", error);
      }
    };
    initTrayMenu();

    // M4 T4：远程状态变化（设置页/托盘开关/通道切换）→ 重建托盘刷新勾选态与地址。
    // 监听放本组件：持有 petOn 与真实键名 tray.petHide/petShow（放 useRemoteEvents
    // 会丢桌宠态、重建出错误标签）；勾选态不经 setState，托盘重建即回环刷新
    const unlistenRemoteChanged = listen("remote-changed", () => void initTrayMenu());

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
      unlistenRemoteChanged.then((fn) => fn());
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
        <div className="flex items-center gap-2">
          {finishedCards.length > 0 && (
            <button
              onClick={clearFinishedCards}
              className="text-muted-foreground hover:bg-muted hover:text-foreground rounded p-1.5 transition-colors"
              title={t("home.clearFinishedCards")}
              aria-label={t("home.clearFinishedCards")}
            >
              <CheckCheck className="h-4 w-4" />
            </button>
          )}
          <NotificationBell />
        </div>
      </div>

      {/* 内容区 */}
      <div className="flex-1 overflow-y-auto">
        {activeTab === "dashboard" ? <SessionGrid sessions={sessions} /> : <ExtensionList />}
      </div>
      {/* 启动校验弹窗（EP2）：外部宠物素材异常时主窗口确认，宠物窗口先行降级 */}
      <PetStartupGuard />
      {/* 遗留 codex 技能链接迁移对话框（spec §4.3）：仅检测命中时可见 */}
      <LegacySkillMigrationDialog migration={legacyMigration} />
    </WindowFrame>
  );
}
