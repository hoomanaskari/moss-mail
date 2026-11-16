/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri expects a fixed port and must not have the dev server clear its output.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // .claude/ holds other sessions' worktrees and coverage/ test reports: neither should reload this app.
    watch: { ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**", "**/.claude/**", "**/coverage/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    target: "es2022",
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
  test: {
    environment: "jsdom",
    setupFiles: ["src/test/setup.ts"],
    include: ["src/**/*.test.{ts,tsx}", "scripts/**/*.test.ts"],
    coverage: {
      include: ["src/**/*.{ts,tsx}", "scripts/lib/**/*.ts"],
      exclude: ["src/**/*.test.{ts,tsx}", "scripts/**/*.test.ts", "src/test/**", "src/main.tsx", "src/data/demo.ts", "src/**/*.d.ts"],
      reporter: ["text", "html", "json-summary"],
      // Floors, not targets: raise them when coverage goes up, never lower them (see AGENTS.md).
      thresholds: { lines: 100, functions: 100, statements: 99.95, branches: 99.91 },
    },
  },
});
