/// <reference types="vitest" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      // Forward /api/* → backend so the dashboard can run without CORS issues locally
      "/api": {
        target: process.env.VITE_API_BASE_URL ?? "http://localhost:3000",
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/api/, ""),
      },
    },
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    coverage: {
      provider: "v8",
      reporter: ["text", "lcov"],
import { defineConfig } from 'vite';

// The dashboard is served by the backend under /ui (issue #1112), so every
// asset URL is emitted relative to that base. Hashed files land in
// dist/assets/, which the backend serves with immutable cache headers.
export default defineConfig({
  base: '/ui/',
  build: {
    outDir: 'dist',
    assetsDir: 'assets',
    // Keep the bundle free of inline scripts so the strict dashboard CSP
    // (script-src 'self') holds.
    modulePreload: { polyfill: false },
  },
  server: {
    // Allow importing ../design/build/tokens.css during `vite dev`.
    fs: { allow: ['..'] },
    proxy: {
      '/v1': { target: process.env.SOROBAN_PULSE_API ?? 'http://localhost:3000', changeOrigin: true },
    },
  },
});
