import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { VitePWA } from "vite-plugin-pwa";

export default defineConfig({
  plugins: [
    react(),
    VitePWA({
      registerType: "autoUpdate",
      strategies: "injectManifest",
      srcDir: "src",
      filename: "sw.ts",
      injectManifest: {
        globPatterns: ["**/*.{js,css,html,svg,png,ico,ttf,woff,woff2,wasm,webmanifest}"],
      },
      includeAssets: ["assets/favicon-32.png"],
      manifest: {
        name: "Pigeon — optical file transfer",
        short_name: "Pigeon",
        description:
          "Turn files into animated QR GIFs and reconstruct them exactly on another device. Fully offline.",
        start_url: "/",
        display: "standalone",
        background_color: "#1784f4",
        theme_color: "#1784f4",
        icons: [
          { src: "/assets/icon-192.png", sizes: "192x192", type: "image/png" },
          { src: "/assets/icon-512.png", sizes: "512x512", type: "image/png" },
          { src: "/assets/icon-512.png", sizes: "512x512", type: "image/png", purpose: "maskable" },
        ],
      },
    }),
  ],
  worker: {
    format: "es",
  },
  build: {
    // WASM bindings use top-level await for instantiation
    target: "es2022",
    sourcemap: true,
  },
  server: {
    host: "127.0.0.1",
    port: 5173,
    strictPort: false,
  },
  preview: {
    host: "127.0.0.1",
    port: 4173,
    strictPort: true,
  },
});
