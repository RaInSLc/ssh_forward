import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// 独立的 Vitest 配置：Vitest 存在本文件时优先于 vite.config.ts，
// 而 `vite build` 完全忽略本文件——因此生产构建路径零改动。
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    // 不启用 globals：测试中显式 import vitest API，无需改动 tsconfig 的 types。
    globals: false,
    restoreMocks: true,
  },
});
