import React from "react";
import ReactDOM from "react-dom/client";
// 移动端入口：纯浏览器页面，零 Tauri API——无 tauri-mock、无路由库、无 QueryProvider
// 不 import 桌面全量样式 src/index.css，仅引入 tailwind 入口
import "./mobile.css";
import App from "./App";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
