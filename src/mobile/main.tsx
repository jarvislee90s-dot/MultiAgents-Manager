import React from "react";
import ReactDOM from "react-dom/client";
// 移动端入口：纯浏览器页面，零 Tauri API——无 tauri-mock、无路由库、无 QueryProvider
// 不 import 桌面全量样式 src/index.css，仅引入 tailwind 入口
import "./mobile.css";
import App from "./App";
import { applyInitialTheme, applyInitialUiConfig, applyUiConfig } from "./theme";
import { fetchUiConfig } from "./api";

// P8f：应用初始主题（读 localStorage/系统偏好 → 设 documentElement 类）。
// 防闪白已由 mobile.html 内联脚本在首帧前完成；此处是权威应用点，React 挂载前执行，
// 保证首屏渲染与 html 类一致（内联脚本被 CSP 或旧 WebView 跳过时的兜底）
applyInitialTheme();

// 2026-10-05 UI 改版：外观配置三级回退（spec §6.2）——镜像先行（与防闪脚本同值，
// React 挂载前生效），服务器值异步覆写（403/失败静默保留镜像，不阻塞首帧）
applyInitialUiConfig();
void fetchUiConfig()
  .then((c) => applyUiConfig(c))
  .catch(() => {});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
