import { fileURLToPath, URL } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'
import { viteSingleFile } from 'vite-plugin-singlefile'

// `npm run dev` proxies the API to a running edgeblend (or the `serve` example):
//   EDGEBLEND_URL=http://127.0.0.1:7878 npm run dev   then open http://localhost:5173/?t=<token>
const target = process.env.EDGEBLEND_URL ?? 'http://127.0.0.1:7878'

export default defineConfig({
  plugins: [vue(), tailwindcss(), viteSingleFile()],
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  server: {
    proxy: { '/api': { target, changeOrigin: true } },
  },
  build: {
    // One self-contained file, embedded in the Rust binary with include_str!.
    outDir: 'dist',
    emptyOutDir: true,
    assetsInlineLimit: 100_000_000,
    cssCodeSplit: false,
    reportCompressedSize: false,
  },
})
