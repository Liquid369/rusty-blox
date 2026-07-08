<script setup>
/* =====================================================================
   APP SHELL — "PIVX MISSION CONTROL" HUD CHROME
   Top telemetry bar (brand reticle + Cmd+K jump console trigger + live
   status cluster), a nav tab rail, the routed viewport, and a footer
   telemetry strip. Data wiring (store / search classifier) is preserved.
   ===================================================================== */
import { ref, onMounted, onBeforeUnmount, computed } from 'vue'
import { useRouter } from 'vue-router'
import { useChainStore } from './store.js'
import { formatCount } from './lib/format.js'
import { formatFiat } from './lib/money.js'
import { isMock } from './api/client.js'
import CommandPalette from './components/CommandPalette.vue'

const router = useRouter()
const chain = useChainStore()
const palette = ref(null)

// live UTC clock + session uptime for the "living network" feel
const clock = ref('')
const uptime = ref(0)
let t = null
let poll = null
let pricePoll = null
function tick() {
  const d = new Date()
  clock.value = d.toISOString().slice(11, 19) + 'Z'
  uptime.value += 1
}

const navItems = [
  { to: '/', label: 'CONTROL', glyph: '⬡' },
  { to: '/blocks', label: 'BLOCKS', glyph: '▦' },
  { to: '/mempool', label: 'MEMPOOL', glyph: '◌' },
  { to: '/masternodes', label: 'NODES', glyph: '⬢' },
  { to: '/governance', label: 'GOVERNANCE', glyph: '⏣' },
  { to: '/analytics', label: 'ANALYTICS', glyph: '◈' },
]

const syncPct = computed(() => (chain.syncPercentage || 0).toFixed(1))

// Skip-link target: move focus into <main> (tabindex=-1) without a hash jump.
function skipToMain() {
  document.getElementById('main')?.focus()
}

onMounted(() => {
  chain.refresh()
  chain.refreshHealth()
  chain.connectLive() // live tip + "since last block" via /ws/blocks (no cache lag)
  tick()
  t = setInterval(tick, 1000)
  // Fallback poll for sync% / network height (and if the WS drops); status
  // caches 5s server-side.
  poll = setInterval(() => chain.refresh(), 15000)
  // Market price (300s server cache + external API) — fetch once, refresh gently.
  chain.refreshPrice()
  pricePoll = setInterval(() => chain.refreshPrice(), 120000)
})
onBeforeUnmount(() => { clearInterval(t); clearInterval(poll); clearInterval(pricePoll); chain.disconnectLive() })
</script>

<template>
  <div class="fx-field" aria-hidden="true"></div>
  <div class="fx-scan" aria-hidden="true"></div>
  <div class="fx-sweep" aria-hidden="true"></div>

  <div class="app">
    <!-- Skip link: first focusable element. @click.prevent (not a bare #main
         href) because the app uses hash-history routing — a hash jump would be
         read as a route change. -->
    <a href="#main" class="skip-link" @click.prevent="skipToMain">Skip to main content</a>

    <!-- ===== TOP TELEMETRY BAR ===== -->
    <header class="topbar">
      <RouterLink to="/" class="brand">
        <span class="reticle" aria-hidden="true"><span class="r-core"></span></span>
        <span class="brand-txt">
          <b>RUSTY<i>//</i>BLOX</b>
          <small>PIVX · MISSION CONTROL</small>
        </span>
      </RouterLink>

      <button
        class="cmdbar"
        aria-label="Search by height, txid, or address — open jump console"
        aria-keyshortcuts="Meta+K Control+K"
        @click="palette && palette.show()"
      >
        <span class="cmdbar-ic" aria-hidden="true">⌕</span>
        <span class="cmdbar-ph">Jump to height · txid · address …</span>
        <kbd>⌘K</kbd>
      </button>

      <div class="telem">
        <div class="t-cell">
          <span class="t-k">SYNC</span>
          <span class="t-v" :class="chain.synced ? 'good' : 'warn'">
            <span class="dot" :class="chain.synced ? 'live' : ''"></span>{{ syncPct }}%
          </span>
        </div>
        <div class="t-cell">
          <span class="t-k">TIP</span>
          <span class="t-v mono">{{ formatCount(chain.height) }}</span>
        </div>
        <div class="t-cell hide-price">
          <span class="t-k">PRICE</span>
          <span class="t-v mono">{{ chain.price ? formatFiat(chain.price.usd) : '—' }}</span>
        </div>
        <div class="t-cell hide-sm">
          <span class="t-k">UTC</span>
          <span class="t-v mono">{{ clock }}</span>
        </div>
        <div class="heartbeat" title="network heartbeat" aria-hidden="true">
          <svg viewBox="0 0 120 28" preserveAspectRatio="none">
            <polyline points="0,14 22,14 30,4 38,24 46,14 70,14 78,8 86,20 94,14 120,14"
              fill="none" stroke="url(#hb)" stroke-width="2" />
            <defs>
              <linearGradient id="hb" x1="0" y1="0" x2="1" y2="0">
                <stop offset="0" stop-color="#c46bff" /><stop offset="1" stop-color="#46e6d0" />
              </linearGradient>
            </defs>
          </svg>
        </div>
      </div>
    </header>

    <!-- ===== NAV TAB RAIL ===== -->
    <nav class="navrail" aria-label="Primary">
      <RouterLink v-for="n in navItems" :key="n.to" :to="n.to" class="tab">
        <span class="tab-g" aria-hidden="true">{{ n.glyph }}</span>{{ n.label }}
      </RouterLink>
      <span class="navrail-fill"></span>
      <span class="navrail-id mono">SESSION {{ String(uptime).padStart(5,'0') }} · {{ isMock ? 'MOCK-NET' : 'LIVE-NET' }}</span>
    </nav>

    <!-- ===== VIEWPORT ===== -->
    <main id="main" class="viewport" tabindex="-1">
      <RouterView />
    </main>

    <!-- ===== FOOTER TELEMETRY ===== -->
    <footer class="foot">
      <span class="dot neon"></span>
      <span class="mono">RUSTYBLOX EXPLORER</span>
      <span class="foot-sep">·</span>
      <span class="mono dim">{{ isMock ? 'offline mock telemetry' : 'live telemetry' }} — units via money.js</span>
      <span class="navrail-fill"></span>
      <span class="mono dim">⌘K JUMP-CONSOLE · / FOCUS</span>
    </footer>

    <CommandPalette ref="palette" />
  </div>
</template>

<style scoped>
.app { position: relative; z-index: 2; min-height: 100%; display: flex; flex-direction: column; }

/* skip link — hidden until keyboard-focused, then drops into view on-brand */
.skip-link {
  position: absolute; left: 50%; top: 8px; z-index: 300;
  transform: translateX(-50%) translateY(-200%);
  padding: 8px 16px; border-radius: var(--radius-md);
  background: var(--neon); color: #0b0716;
  font-family: var(--font-mono); font-size: 12px; font-weight: 700; letter-spacing: 0.1em;
  text-decoration: none; box-shadow: var(--glow-sm); transition: transform .15s;
}
.skip-link:focus { transform: translateX(-50%) translateY(0); }

/* ---- topbar ---- */
.topbar {
  display: flex; align-items: center; gap: var(--space-4);
  height: var(--topbar-h); padding: 0 var(--space-5);
  background: var(--rail-grad);
  border-bottom: 1px solid var(--glass-edge);
  box-shadow: 0 1px 0 rgba(196,107,255,0.08), 0 8px 30px rgba(0,0,0,0.4);
  position: sticky; top: 0; z-index: 50; backdrop-filter: blur(10px);
}
.brand { display: flex; align-items: center; gap: 11px; }
.reticle {
  width: 26px; height: 26px; border-radius: 7px; position: relative;
  border: 1.5px solid var(--neon); box-shadow: var(--glow-sm), inset 0 0 10px rgba(196,107,255,0.25);
  display: grid; place-items: center; background: rgba(196,107,255,0.06);
}
.r-core { width: 9px; height: 9px; border-radius: 50%; background: var(--holo); box-shadow: var(--glow-sm); animation: pulse-n 2.4s infinite; }
.brand-txt { display: flex; flex-direction: column; line-height: 1.05; }
.brand-txt b { font-family: var(--font-mono); font-size: 15px; letter-spacing: 0.5px; color: var(--text); }
.brand-txt b i { color: var(--neon); font-style: normal; text-shadow: var(--glow-xs); }
.brand-txt small { font-family: var(--font-mono); font-size: 8.5px; letter-spacing: 0.22em; color: var(--text-dim); }

.cmdbar {
  flex: 1; max-width: 560px; display: flex; align-items: center; gap: 10px;
  padding: 9px 14px; cursor: text;
  background: var(--glass); border: 1px solid var(--hud-line); border-radius: var(--radius-md);
  color: var(--text-dim); font: inherit; transition: all .15s;
}
.cmdbar:hover { border-color: var(--glass-edge); box-shadow: var(--glow-xs); }
.cmdbar-ic { color: var(--neon); font-size: 15px; }
/* min-width:0 + nowrap/ellipsis: never wrap the hint — a wrapped placeholder
   grew the fixed-height topbar and spilled over the nav rail on the Fold. */
.cmdbar-ph { flex: 1; min-width: 0; text-align: left; font-family: var(--font-mono); font-size: 12.5px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.cmdbar kbd { font-family: var(--font-mono); font-size: 10px; padding: 2px 7px; border-radius: 5px; border: 1px solid var(--hud-line); color: var(--text-muted); background: var(--glass-2); }

.telem { display: flex; align-items: center; gap: var(--space-4); }
.t-cell { display: flex; flex-direction: column; align-items: flex-end; line-height: 1.1; }
.t-k { font-family: var(--font-mono); font-size: 8.5px; letter-spacing: 0.18em; color: var(--text-dim); }
.t-v { font-family: var(--font-mono); font-size: 13px; font-weight: 700; color: var(--text); display: flex; align-items: center; gap: 6px; }
.t-v.good { color: var(--success); }
.t-v.warn { color: var(--warn); }
.heartbeat { width: 96px; height: 26px; opacity: 0.9; }
.heartbeat svg { width: 100%; height: 100%; filter: drop-shadow(0 0 4px rgba(196,107,255,0.5)); }

/* ---- nav rail ---- */
.navrail {
  display: flex; align-items: center; gap: 4px; padding: 0 var(--space-5);
  height: var(--tabbar-h);
  background: rgba(10,6,20,0.6); border-bottom: 1px solid var(--hud-line);
  position: sticky; top: var(--topbar-h); z-index: 40; backdrop-filter: blur(8px);
}
.tab {
  position: relative; display: flex; align-items: center; gap: 8px;
  padding: 0 16px; height: 100%;
  font-family: var(--font-mono); font-size: 11.5px; font-weight: 600; letter-spacing: 0.14em;
  color: var(--text-dim); text-decoration: none; transition: color .15s;
}
.tab .tab-g { font-size: 13px; opacity: 0.7; }
.tab:hover { color: var(--text-muted); }
.tab.router-link-exact-active { color: var(--neon); text-shadow: var(--glow-xs); }
.tab.router-link-exact-active::after {
  content: ""; position: absolute; left: 10px; right: 10px; bottom: -1px; height: 2px;
  background: var(--holo); box-shadow: var(--glow-sm); border-radius: 2px;
}
.tab.router-link-exact-active .tab-g { opacity: 1; }
.navrail-fill { flex: 1; }
.navrail-id { font-size: 9.5px; letter-spacing: 0.16em; color: var(--text-dim); }

/* ---- viewport ---- */
.viewport { flex: 1; width: 100%; max-width: 1320px; margin: 0 auto; padding: var(--space-5) var(--space-5) var(--space-7); }

/* ---- footer ---- */
.foot {
  display: flex; align-items: center; gap: 10px; padding: 10px var(--space-5);
  border-top: 1px solid var(--hud-line); background: rgba(8,5,16,0.7);
  font-size: 10.5px; letter-spacing: 0.08em; color: var(--text-muted);
}
.foot .mono { font-size: 10.5px; }
.foot-sep { color: var(--text-dim); }

/* Fold / small tablet (720-900): the topbar is still cramped there, so drop the
   decorative 96px heartbeat to give the search bar room. */
@media (max-width: 900px) {
  .heartbeat { display: none; }
  .hide-price { display: none; }  /* price ticker off in the Fold/mobile range (shown on Dashboard + Governance) */
}

@media (max-width: 720px) {
  .hide-sm { display: none; }
  .brand-txt small { display: none; }
  .cmdbar-ph { display: none; }
  .topbar { gap: var(--space-3); padding: 0 var(--space-4); }
  .cmdbar { flex: 0 1 auto; padding: 9px 11px; }
  .cmdbar kbd { display: none; }   /* no ⌘ key on mobile; frees space so TIP fits */
  .telem { gap: var(--space-3); }
  /* nav rail: scroll the tabs horizontally instead of overflowing the page */
  .navrail { overflow-x: auto; scrollbar-width: none; padding: 0 var(--space-4); }
  .navrail::-webkit-scrollbar { display: none; }
  .tab { flex: none; padding: 0 13px; }
  .navrail-fill, .navrail-id { display: none; }
  /* tighter page gutters on phones */
  .viewport { padding: var(--space-4) var(--space-4) var(--space-6); }
}
</style>
