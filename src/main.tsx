import "./tauri-mock";
import React, { lazy, Suspense, useEffect } from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./index.css";
import "./i18n";

const HomePage = lazy(() => import("./pages/home"));
const AboutPage = lazy(() => import("./pages/about"));
const SettingsPage = lazy(() => import("./pages/settings"));

const pageMap = {
  "/": HomePage,
  "/about": AboutPage,
  "/settings": SettingsPage,
};

const pathname = window.location.pathname;
const PageComponent = pageMap[pathname as keyof typeof pageMap] ?? HomePage;

function AppWrapper() {
  useEffect(() => {
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
    <Suspense fallback={null}>
      <AppWrapper />
    </Suspense>
  </React.StrictMode>
);
