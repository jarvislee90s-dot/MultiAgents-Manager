import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "path";

// 移动端第二入口：独立构建到 dist-mobile/（由 rust-embed 嵌入 axum 伺服）
export default defineConfig({
  // 页面由 axum 伺服在 /m 路径下：资源必须以 /m/ 为前缀，否则绝对路径 /assets/* 会 404 白屏
  base: "/m/",
  // 移动页不用桌面静态资源（public/ 下的 pet 动效与提示音约 9.5MB）：
  // 不能指回 public/，否则桌面资源会被拷进 dist-mobile 再被 rust-embed 嵌进二进制；
  // 专用目录只放 PWA 资产（manifest-mam.json + icon-mobile.png），构建时拷入 dist-mobile 根
  publicDir: "public-mobile",
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": path.resolve(__dirname, "./src") } },
  build: {
    outDir: "dist-mobile",
    // 保留 dist-mobile/.gitkeep 占位（rust-embed 编译依赖目录存在，Task 1 已入库）；
    // 代价是旧 hash 资源残留，已接受
    emptyOutDir: false,
    rollupOptions: { input: path.resolve(__dirname, "mobile.html") },
  },
});
