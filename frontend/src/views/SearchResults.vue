<script setup>
/* =====================================================================
   SEARCH RESULTS — /search classifier. The endpoint is a router, not a
   render target: it returns an internally-tagged enum (Block / Transaction
   / Address / XPub / NotFound). We echo the classified result as a card
   linking to the authoritative entity page (Address.balance is always null
   here — the address page fetches the real balance).
   ===================================================================== */
import { ref, onMounted, watch, computed } from 'vue'
import { getSearch } from '../api/client.js'
import { truncateHash, formatCount } from '../lib/format.js'
import HudPanel from '../components/HudPanel.vue'

const props = defineProps({ query: { type: String, required: true } })
const result = ref(null)

async function load() { result.value = null; result.value = await getSearch(props.query) }
onMounted(load)
watch(() => props.query, load)

// Map the enum tag to a route + display metadata.
const card = computed(() => {
  const r = result.value
  if (!r) return null
  switch (r.type) {
    case 'Block': return { glyph: '▦', kind: 'BLOCK', to: `/block/${r.height}`, title: `Block #${formatCount(r.height)}`, sub: truncateHash(r.hash, 16, 12) }
    case 'Transaction': return { glyph: '⇄', kind: 'TRANSACTION', to: `/tx/${r.txid}`, title: 'Transaction', sub: `${truncateHash(r.txid, 14, 10)}${r.block_height != null ? ` · block ${formatCount(r.block_height)}` : ''}` }
    case 'Address': return { glyph: '⬡', kind: 'ADDRESS', to: `/address/${r.address}`, title: 'Address', sub: r.address }
    case 'XPub': return { glyph: '⌖', kind: 'XPUB ACCOUNT', to: `/xpub/${r.xpub}`, title: 'Extended public key', sub: truncateHash(r.xpub, 18, 12) }
    default: return null
  }
})
</script>

<template>
  <div class="page">
    <div class="page-head">
      <div>
        <div class="eyebrow">UNIVERSAL LOOKUP · /search</div>
        <h1 class="page-title">Search</h1>
      </div>
      <div class="head-live"><span class="pill neon mono">⌘K JUMP CONSOLE</span></div>
    </div>

    <HudPanel>
      <div class="q-row">
        <span class="eyebrow">QUERY</span>
        <span class="mono q-val">{{ query }}</span>
      </div>
    </HudPanel>

    <div v-if="!result" class="loading" style="margin-top: var(--space-4)">classifying…</div>

    <template v-else-if="card">
      <h2 class="section-title">Match found</h2>
      <RouterLink :to="card.to" class="result">
        <span class="result-glyph" aria-hidden="true">{{ card.glyph }}</span>
        <div class="result-body">
          <span class="pill cyan mono">{{ card.kind }}</span>
          <div class="result-title">{{ card.title }}</div>
          <div class="result-sub mono dim">{{ card.sub }}</div>
        </div>
        <span class="result-go pill neon">OPEN ›</span>
      </RouterLink>
    </template>

    <template v-else>
      <h2 class="section-title">No results</h2>
      <HudPanel title="NOT FOUND" id="NotFound">
        <p class="nf mono">
          “{{ query }}” did not classify as a block height, block/tx hash, address, or xpub.
        </p>
        <p class="nf dim mono">
          Try a numeric height, a 64-hex hash, a D/S/6/7/E address, or an xpub. Open the
          <RouterLink to="/">jump console</RouterLink> with ⌘K.
        </p>
      </HudPanel>
    </template>
  </div>
</template>

<style scoped>
.head-live { display: flex; align-items: center; gap: 10px; margin-left: auto; }
.q-row { display: flex; flex-direction: column; gap: 6px; }
.q-val { font-size: 14px; color: var(--text); word-break: break-all; }
.result {
  display: flex; align-items: center; gap: var(--space-4); padding: var(--space-5);
  background: var(--panel-grad); border: 1px solid var(--glass-edge-strong);
  border-radius: var(--radius-lg); text-decoration: none; transition: box-shadow .15s, transform .15s;
}
.result:hover { box-shadow: var(--glow-md); transform: translateY(-2px); }
.result-glyph { font-size: 34px; color: var(--neon); text-shadow: var(--glow-sm); width: 48px; text-align: center; }
.result-body { flex: 1; display: flex; flex-direction: column; gap: 6px; }
.result-title { font-family: var(--font-mono); font-size: 18px; font-weight: 700; color: var(--text); }
.result-sub { font-size: 12px; word-break: break-all; }
.result-go { align-self: center; }
.nf { font-size: 13px; color: var(--text-muted); margin: 0 0 10px; }
.nf.dim { font-size: 12px; }
.nf a { color: var(--neon); }
</style>
