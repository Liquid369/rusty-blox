<script setup>
/* On-brand HUD 404 — catch-all for unknown routes. */
import { useRoute } from 'vue-router'
const route = useRoute()
</script>

<template>
  <div class="page nf-page">
    <div class="nf-reticle" aria-hidden="true">
      <span class="nf-code">404</span>
      <span class="nf-ring"></span>
    </div>
    <h1 class="page-title">Signal lost</h1>
    <p class="nf-msg mono dim">
      No telemetry for <span class="neon-text">{{ route.fullPath }}</span> — the coordinate is off the chart.
    </p>
    <div class="nf-actions">
      <RouterLink to="/" class="gbtn on">⬡ MISSION CONTROL</RouterLink>
      <RouterLink to="/blocks" class="gbtn">▦ BLOCKS</RouterLink>
      <RouterLink to="/masternodes" class="gbtn">⬢ MASTERNODES</RouterLink>
      <RouterLink to="/governance" class="gbtn">⏣ GOVERNANCE</RouterLink>
    </div>
    <p class="nf-hint mono dim">Press ⌘K to open the jump console.</p>
  </div>
</template>

<style scoped>
.nf-page { display: flex; flex-direction: column; align-items: center; text-align: center; padding-top: 10vh; gap: var(--space-4); }
.nf-reticle { position: relative; width: 140px; height: 140px; display: grid; place-items: center; margin-bottom: var(--space-3); }
.nf-code { font-family: var(--font-mono); font-size: 44px; font-weight: 700; color: var(--neon); text-shadow: var(--glow-md); z-index: 1; }
.nf-ring { position: absolute; inset: 0; border: 2px solid var(--glass-edge-strong); border-radius: 50%; box-shadow: var(--glow-sm), inset 0 0 30px rgba(196,107,255,0.18); border-top-color: var(--cyan); animation: spin 8s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
.nf-msg { font-size: 13px; max-width: 520px; }
.nf-actions { display: flex; flex-wrap: wrap; gap: 10px; justify-content: center; margin-top: var(--space-3); }
.nf-actions .gbtn { text-decoration: none; }
.nf-hint { font-size: 11px; margin-top: var(--space-3); }
</style>
