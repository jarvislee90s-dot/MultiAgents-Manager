import React from "react";
import ReactDOM from "react-dom/client";
// 移动端入口：纯浏览器页面，零 Tauri API——无 tauri-mock、无路由库、无 QueryProvider
// 不 import 桌面全量样式 src/index.css，仅引入 tailwind 入口
import "./mobile.css";
import App from "./App";
import { applyInitialTheme } from "./theme";

// P8f：应用初始主题（读 localStorage/系统偏好 → 设 documentElement 类）。
// 防闪白已由 mobile.html 内联脚本在首帧前完成；此处是权威应用点，React 挂载前执行，
// 保证首屏渲染与 html 类一致（内联脚本被 CSP 或旧 WebView 跳过时的兜底）
applyInitialTheme();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
