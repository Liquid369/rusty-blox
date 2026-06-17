import { fileURLToPath, URL } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()],
  esbuild: {
    drop: ['console', 'debugger']
  },
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url))
    }
  },
  server: {
    port: 3001,
    host: true, // Listen on all addresses
    allowedHosts: ['.trycloudflare.com'], // Allow Cloudflare tunnel domains
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:3005',
        changeOrigin: true
      },
      '/ws': {
        target: 'ws://127.0.0.1:3005',
        ws: true
      }
    }
  },
  build: {
    outDir: 'dist',
    sourcemap: false,
    rollupOptions: {
      output: {
        // Function form required by vite 8's rolldown bundler (object map is rejected).
        manualChunks(id) {
          if (!id.includes('node_modules')) return
          if (/[\\/]node_modules[\\/](echarts|zrender|vue-echarts)[\\/]/.test(id)) return 'charts'
          if (/[\\/]node_modules[\\/](@vue[\\/]|vue[\\/]|vue-router[\\/]|pinia[\\/]|axios[\\/])/.test(id)) return 'vendor'
        }
      }
    }
  }
})
