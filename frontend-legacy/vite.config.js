import { fileURLToPath, URL } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url))
    }
  },
  server: {
    port: 3000,
    host: true, // Listen on all addresses
    allowedHosts: ['.trycloudflare.com'], // Allow Cloudflare tunnel domains
    proxy: {
      '/api': {
        target: 'http://localhost:3005',
        changeOrigin: true
      },
      '/ws': {
        target: 'ws://localhost:3005',
        ws: true
      }
    }
  },
  build: {
    outDir: 'dist',
    sourcemap: true,
    rollupOptions: {
      output: {
        manualChunks: {
          'vendor': ['vue', 'vue-router', 'pinia', 'axios'],
          'charts': ['echarts', 'vue-echarts']
        }
      }
    }
  }
})
