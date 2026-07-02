import { fileURLToPath, URL } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// NEUTRAL base config for the redesign competition.
// base './' so a built prototype can be served from any sub-path.
// Port 5180 (strictPort:false so it hops if busy) keeps it off the legacy app's 3001.
export default defineConfig({
  plugins: [vue()],
  base: './',
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url))
    }
  },
  server: {
    port: 5180,
    strictPort: false,
    // When pointing the client at a real backend instead of the mock,
    // set isMock=false in src/api/client.js and rely on this proxy.
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:3005',
        changeOrigin: true
      },
      '/ws': {
        target: 'http://127.0.0.1:3005',
        changeOrigin: true,
        ws: true
      }
    }
  },
  build: {
    outDir: 'dist',
    sourcemap: false
  }
})
