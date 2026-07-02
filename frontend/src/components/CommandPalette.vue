<script setup>
/* =====================================================================
   COMMAND PALETTE (Cmd/Ctrl+K) — universal HUD jump console.
   Mirrors the backend /search classifier:
     digits -> block, 64-hex -> tx, D/S/6/7/E… -> address.
   Also exposes quick nav destinations + sample deep-links.
   ===================================================================== */
import { ref, computed, watch, onMounted, onBeforeUnmount, nextTick } from 'vue'
import { useRouter } from 'vue-router'

const router = useRouter()
const open = ref(false)
const q = ref('')
const inputEl = ref(null)
const sel = ref(0)
let lastFocused = null   // restore focus to the trigger on close

const NAV = [
  { kind: 'NAV', label: 'Mission Control', hint: 'dashboard / heartbeat', to: '/' },
  { kind: 'NAV', label: 'Blocks', hint: 'recent block ledger', to: '/blocks' },
  { kind: 'NAV', label: 'Mempool', hint: 'pending transactions', to: '/mempool' },
  { kind: 'NAV', label: 'Masternodes', hint: 'node network · /mnlist', to: '/masternodes' },
  { kind: 'NAV', label: 'Governance', hint: 'budget proposals · superblock', to: '/governance' },
  { kind: 'NAV', label: 'Analytics Deck', hint: 'HODL · wealth · staking', to: '/analytics' },
  { kind: 'SAMPLE', label: 'Block #5475000', hint: 'block-detail', to: '/block/5475000' },
  { kind: 'SAMPLE', label: 'Sample transaction', hint: 'tx value-flow', to: '/tx/coinstake' },
  { kind: 'SAMPLE', label: 'Whale address (rich #1)', hint: 'address · utxo', to: '/address/DU8gPC5mh4KxWJARQRxoESFark2jAguBr5' },
  { kind: 'SAMPLE', label: 'Sample xpub account', hint: 'HD account aggregate', to: '/xpub/xpub6CUGRUonZSQ4TWtTMmzXdrXDtyPWKiKi3qJ' },
]

function classify(s) {
  if (!s) return null
  if (/^\d+$/.test(s)) return { type: 'BLOCK', to: `/block/${s}`, label: `Block #${s}`, hint: 'height → block-detail' }
  if (/^[0-9a-fA-F]{64}$/.test(s)) return { type: 'TX', to: `/tx/${s}`, label: 'Transaction', hint: '64-hex → tx detail' }
  if (/^(xpub)/i.test(s)) return { type: 'XPUB', to: `/xpub/${s}`, label: 'Extended pubkey', hint: 'xpub → account' }
  if (/^[DS67E]/.test(s)) return { type: 'ADDRESS', to: `/address/${s}`, label: 'Address', hint: 'base58 → account' }
  return { type: 'ADDRESS', to: `/address/${s}`, label: 'Lookup', hint: 'route to address' }
}

const parsed = computed(() => classify(q.value.trim()))

const results = computed(() => {
  const list = []
  if (parsed.value) list.push({ ...parsed.value, primary: true })
  const term = q.value.trim().toLowerCase()
  for (const n of NAV) {
    if (!term || n.label.toLowerCase().includes(term) || n.hint.toLowerCase().includes(term)) list.push(n)
  }
  return list
})

watch(results, () => { sel.value = 0 })

function go(item) {
  if (!item) return
  router.push(item.to)
  close()
}
function show() {
  lastFocused = document.activeElement
  open.value = true
  q.value = ''
  sel.value = 0
  nextTick(() => inputEl.value && inputEl.value.focus())
}
function close() {
  open.value = false
  // return focus to whatever opened the palette (sane tab order)
  if (lastFocused && lastFocused.focus) lastFocused.focus()
}

function onKeydown(e) {
  const meta = e.metaKey || e.ctrlKey
  if (meta && e.key.toLowerCase() === 'k') { e.preventDefault(); open.value ? close() : show(); return }
  if (e.key === '/' && !open.value && !/input|textarea/i.test(e.target.tagName)) { e.preventDefault(); show(); return }
  if (!open.value) return
  if (e.key === 'Escape') { close(); return }
  // focus trap: only the input is focusable here, so keep Tab inside the dialog
  if (e.key === 'Tab') { e.preventDefault(); inputEl.value && inputEl.value.focus(); return }
  if (e.key === 'ArrowDown') { e.preventDefault(); sel.value = Math.min(sel.value + 1, results.value.length - 1) }
  if (e.key === 'ArrowUp') { e.preventDefault(); sel.value = Math.max(sel.value - 1, 0) }
  if (e.key === 'Enter') { e.preventDefault(); go(results.value[sel.value]) }
}

onMounted(() => window.addEventListener('keydown', onKeydown))
onBeforeUnmount(() => window.removeEventListener('keydown', onKeydown))

// allow the topbar trigger to open it
defineExpose({ show })
</script>

<template>
  <teleport to="body">
    <transition name="cmd">
      <div v-if="open" class="cmd-scrim" @click.self="close">
        <div class="cmd-box panel hero" role="dialog" aria-modal="true" aria-label="Jump console">
          <div class="cmd-input-row">
            <span class="cmd-prompt" aria-hidden="true">›_</span>
            <input
              ref="inputEl"
              v-model="q"
              class="cmd-input mono"
              role="combobox"
              aria-label="Search by height, transaction id, or address"
              aria-expanded="true"
              aria-controls="cmd-results"
              :aria-activedescendant="results.length ? `cmd-opt-${sel}` : undefined"
              placeholder="Jump to height · txid · address · xpub …"
              spellcheck="false"
              autocomplete="off"
            />
            <span v-if="parsed" class="pill neon">{{ parsed.type }}</span>
            <kbd class="cmd-kbd">ESC</kbd>
          </div>
          <ul id="cmd-results" class="cmd-list" role="listbox" aria-label="Jump results">
            <li
              v-for="(r, i) in results"
              :id="`cmd-opt-${i}`"
              :key="i"
              class="cmd-item"
              role="option"
              :aria-selected="i === sel"
              :class="{ on: i === sel, primary: r.primary }"
              @mouseenter="sel = i"
              @click="go(r)"
            >
              <span class="cmd-glyph" aria-hidden="true">{{ r.primary ? '⏎' : (r.kind === 'NAV' ? '◆' : '↗') }}</span>
              <span class="cmd-label">{{ r.label }}</span>
              <span class="cmd-hint">{{ r.hint }}</span>
              <span v-if="r.primary" class="pill cyan">GO</span>
            </li>
            <li v-if="!results.length" class="cmd-empty" role="option" aria-disabled="true">No match — type a height, txid, or address.</li>
          </ul>
          <div class="cmd-foot">
            <span><kbd class="cmd-kbd">↑↓</kbd> navigate</span>
            <span><kbd class="cmd-kbd">⏎</kbd> open</span>
            <span><kbd class="cmd-kbd">⌘K</kbd> toggle</span>
            <span class="cmd-foot-id">RUSTYBLOX // JUMP-CONSOLE</span>
          </div>
        </div>
      </div>
    </transition>
  </teleport>
</template>

<style scoped>
.cmd-scrim {
  position: fixed; inset: 0; z-index: 200;
  background: rgba(5,3,10,0.62); backdrop-filter: blur(6px);
  display: flex; align-items: flex-start; justify-content: center; padding-top: 12vh;
}
.cmd-box { width: min(640px, 92vw); overflow: hidden; box-shadow: var(--glow-lg), 0 30px 80px rgba(0,0,0,0.6); }
.cmd-input-row { display: flex; align-items: center; gap: 12px; padding: 16px 18px; border-bottom: 1px solid var(--hud-line); }
/* the input auto-focuses on open; frame the row so focus is always visible */
.cmd-input-row:focus-within { box-shadow: inset 0 0 0 1px var(--glass-edge-strong); }
.cmd-prompt { color: var(--neon); font-family: var(--font-mono); font-weight: 700; text-shadow: var(--glow-xs); }
/* outline removed (not stripped to nothing): :focus-within frame above +
   the global :focus-visible cyan ring provide the visible focus state. */
.cmd-input { flex: 1; background: transparent; border: none; color: var(--text); font-size: 16px; }
.cmd-input::placeholder { color: var(--text-dim); }
.cmd-kbd { font-family: var(--font-mono); font-size: 9.5px; padding: 2px 6px; border-radius: 4px; border: 1px solid var(--hud-line); color: var(--text-dim); background: var(--glass-2); }
.cmd-list { list-style: none; margin: 0; padding: 8px; max-height: 46vh; overflow: auto; }
.cmd-item { display: flex; align-items: center; gap: 12px; padding: 11px 12px; border-radius: var(--radius-md); cursor: pointer; }
.cmd-item .cmd-glyph { color: var(--text-dim); width: 16px; text-align: center; }
.cmd-item .cmd-label { color: var(--text); font-family: var(--font-mono); font-size: 13px; }
.cmd-item .cmd-hint { margin-left: auto; color: var(--text-dim); font-family: var(--font-mono); font-size: 10.5px; letter-spacing: 0.06em; }
.cmd-item.on { background: rgba(196,107,255,0.12); box-shadow: inset 2px 0 0 var(--neon); }
.cmd-item.on .cmd-glyph { color: var(--neon); }
.cmd-item.primary .cmd-label { color: var(--neon); }
.cmd-empty { padding: 16px; color: var(--text-dim); font-family: var(--font-mono); font-size: 12px; text-align: center; }
.cmd-foot { display: flex; gap: 16px; align-items: center; padding: 10px 16px; border-top: 1px solid var(--hud-line); font-family: var(--font-mono); font-size: 10px; color: var(--text-dim); letter-spacing: 0.08em; }
.cmd-foot-id { margin-left: auto; color: var(--neon-soft); opacity: 0.7; }

.cmd-enter-active, .cmd-leave-active { transition: opacity .16s ease; }
.cmd-enter-from, .cmd-leave-to { opacity: 0; }
.cmd-enter-active .cmd-box { animation: cmdpop .2s ease; }
@keyframes cmdpop { from { transform: translateY(-12px) scale(.98); opacity: 0; } to { transform: none; opacity: 1; } }
</style>
