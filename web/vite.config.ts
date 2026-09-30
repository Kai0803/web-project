import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { fileURLToPath, URL } from 'node:url'

export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  build: { outDir: 'dist', emptyOutDir: true },
  server: {
    // 開發時把 API 轉給後端，維持同源，cookie 與 CSRF 的行為才與正式環境一致
    proxy: { '/api': 'http://localhost:8080' },
  },
})
