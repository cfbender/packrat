import { defineConfig } from "vite";
import { VitePWA } from "vite-plugin-pwa";

export default defineConfig({
  server: {
    port: 5174,
    proxy: {
      "/api": {
        target: `http://localhost:${process.env.PORT ?? 4100}`,
        changeOrigin: true,
      },
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
  plugins: [
    VitePWA({
      registerType: "prompt",
      includeAssets: ["favicon.svg", "icons/apple-touch-icon.png"],
      manifest: {
        name: "packrat",
        short_name: "packrat",
        description: "Booster pack and sealed product simulator with expected value.",
        theme_color: "#14121a",
        background_color: "#14121a",
        display: "standalone",
        start_url: "/",
        icons: [
          { src: "/icons/icon-192.png", sizes: "192x192", type: "image/png" },
          { src: "/icons/icon-512.png", sizes: "512x512", type: "image/png" },
          { src: "/icons/icon-512-maskable.png", sizes: "512x512", type: "image/png", purpose: "maskable" },
        ],
      },
      workbox: {
        navigateFallback: "/index.html",
        navigateFallbackDenylist: [/^\/api\//],
        runtimeCaching: [
          {
            urlPattern: ({ url }) => url.origin === "https://cards.scryfall.io",
            handler: "CacheFirst",
            options: {
              cacheName: "scryfall-images",
              expiration: { maxEntries: 1500, maxAgeSeconds: 60 * 60 * 24 * 30 },
              cacheableResponse: { statuses: [0, 200] },
            },
          },
          {
            urlPattern: ({ url }) => url.pathname === "/api/products" || url.pathname === "/api/ev",
            handler: "NetworkFirst",
            options: {
              cacheName: "packrat-api",
              networkTimeoutSeconds: 8,
              expiration: { maxEntries: 20, maxAgeSeconds: 60 * 60 * 24 },
            },
          },
        ],
      },
    }),
  ],
});
