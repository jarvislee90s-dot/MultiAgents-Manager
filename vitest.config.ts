import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import path from "path";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    globals: true,
    include: ["tests/**/*.test.{ts,tsx}"],
    setupFiles: ["./tests/setup.ts"],
    // Node >= 25 的实验性 Web Storage 会在 globalThis 上定义返回 undefined 的 localStorage getter，
    // 遮蔽 jsdom 实现；关闭后由 jsdom 原生接管（消除 ExperimentalWarning）
    execArgv: ["--no-experimental-webstorage"],
    coverage: {
      provider: "v8",
      reporter: ["text", "html"],
      exclude: ["node_modules/", "tests/", "src/lib/api/", "src/components/ui/"],
    },
  },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
});
