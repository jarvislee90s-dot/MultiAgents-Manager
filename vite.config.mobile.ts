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
    // 清空重建（终审修复轮）：emptyOutDir:false 曾让旧 hash 产物无限积累（实测 5 js/3 css
    // ~1.1MB），rust-embed 会把陈旧产物全量嵌进二进制。.gitkeep 占位不受清空影响：
    // publicDir 拷贝阶段会把 public-mobile/.gitkeep 拷回 dist-mobile/.gitkeep
    // （0 字节内容不变 → git 无删除），rust-embed 编译依赖的入库占位得以保留
    emptyOutDir: true,
    rollupOptions: { input: path.resolve(__dirname, "mobile.html") },
  },
});
