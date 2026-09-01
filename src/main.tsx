import "./tauri-mock";
import React, { lazy, Suspense, useEffect } from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./index.css";
import "./i18n";
import { QueryClientProvider } from "@tanstack/react-query";
import { queryClient } from "@/lib/query/queryClient";

const HomePage = lazy(() => import("./pages/home"));
const AboutPage = lazy(() => import("./pages/about"));
const SettingsPage = lazy(() => import("./pages/settings"));

const NotificationPage = lazy(() => import("./pages/notification"));
const PetPage = lazy(() => import("./pages/pet"));

const pageMap = {
  "/": HomePage,
  "/about": AboutPage,
  "/settings": SettingsPage,
};

const pathname = window.location.pathname;
// 通知浮窗与宠物窗口通过 hash 路由分流，与主窗口页面互不干扰
const isNotificationWindow = window.location.hash === "#/notification";
const isPetWindow = window.location.hash === "#/pet";
const PageComponent = isNotificationWindow
  ? NotificationPage
  : isPetWindow
    ? PetPage
    : (pageMap[pathname as keyof typeof pageMap] ?? HomePage);

function AppWrapper() {
  useEffect(() => {
    // 通知浮窗的显隐由 notification:new 事件驱动，宠物窗口由显隐状态驱动，创建时保持隐藏（避免空白窗抢显示）
    if (isNotificationWindow || isPetWindow) return;
    // Show window after React is ready (safe in browser too)
    try {
      getCurrentWindow().show();
    } catch {
      // Not in Tauri environment — ignore
    }
  }, []);

  return <PageComponent />;
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <Suspense fallback={null}>
        <AppWrapper />
      </Suspense>
    </QueryClientProvider>
  </React.StrictMode>
);
