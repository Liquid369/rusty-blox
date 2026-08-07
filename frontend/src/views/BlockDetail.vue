<script setup>
/* =====================================================================
   BLOCK DETAIL — header telemetry + tx-type distribution donut +
   per-transaction value-flow (vin → vout) tables.
   UNITS: tx-level value_in/value_out/fees/reward = PIV (formatPiv);
          per-vin/vout value = satoshi FLOAT (formatSats).
   ===================================================================== */
import { ref, watch, onMounted, computed } from 'vue'
import { getBlockDetail } from '../api/client.js'
import { formatSats, formatPiv } from '../lib/money.js'
import { formatDateTime, truncateHash, formatDifficulty, formatCount, timeAgo, compactNumber } from '../lib/format.js'
import { baseOption, palette, hexA } from '../lib/chart.js'
import { coinstakeInputAddresses, coinstakeInputValueSat } from '../lib/coinstake.js'
import { shieldShape } from '../lib/shield.js'
import EChart from '../components/EChart.vue'
import HudPanel from '../components/HudPanel.vue'
import Stat from '../components/Stat.vue'

const props = defineProps({ height: { type: [String, Number], required: true } })
const block = ref(null)
const err = ref(null)

async function load() {
  err.value = null; block.value = null
  try { block.value = await getBlockDetail(props.height) }
  catch (e) { err.value = e.message }
}
onMounted(load)
watch(() => props.height, load)

// A tx carries Sapling spends/outputs → shielded activity.
const isShielded = (t) => !!t.sapling && ((t.sapling.shielded_spend_count || 0) > 0 || (t.sapling.shielded_output_count || 0) > 0)
// Sapling txs are typed by direction: shielding (t→s), de-shielding (s→t),
// shielded (s→s). Coinbase/coinstake are never reclassified; the degenerate
// vb==0-with-transparent-io record falls back to transparent + SHIELDED badge.
const txType = (t) => {
  const x = t.tx_type || 'transparent'
  if (x === 'coinbase' || x === 'coinstake') return x
  return shieldShape(t) || (x === 'normal' ? 'transparent' : x)
}
const SHIELD_TYPES = ['shielding', 'de-shielding', 'shielded']
const TYPE_COLOR = { coinbase: '#ffcf5c', coinstake: '#c46bff', transparent: '#46e6d0', shielding: '#ff8fe0', 'de-shielding': '#e03fa8', shielded: '#ff5fd0' }
// Directed vb for the pool rows / aggregates: 0 for s→s (its vb is the fee,
// not a transfer — printing it as a pool amount would misstate private values).
const vbOf = (t) => { const s = shieldShape(t); return s && s !== 'shielded' ? (t.sapling.value_balance || 0) : 0 }

// Block reward (total minted, PIV float) -> satoshi, for recovering the value the
// cold-staker put in (the backend leaves P2CS coinstake inputs blank).
const rewardSat = computed(() => block.value ? Math.round((block.value.reward || 0) * 1e8) : null)
// [staker(S), owner(D)] to show for an unresolved cold-stake input, else null.
const vinCold = (t, vin) => coinstakeInputAddresses(t, vin)
// Consumed stake (satoshi) for a cold coinstake's single input, else null. Guarded so
// a plain single-input payment never gets value_out − reward applied to it.
const vinColdValueSat = (t) => {
  if (!(t.vin || []).some((v) => vinCold(t, v))) return null
  return coinstakeInputValueSat(Math.round((t.value_out || 0) * 1e8), rewardSat.value, t.vin.length)
}
// Effective tx value-in/out (PIV floats): recovered cold-stake input, plus the
// shield-pool leg (vb>0 enters as input, vb<0 leaves as output) so the card's
// in − out = fee identity balances on screen for shielding/de-shielding txs.
const txIn = (t) => { const s = vinColdValueSat(t); const base = s != null ? s / 1e8 : (t.value_in || 0); return base + Math.max(vbOf(t), 0) }
const txOut = (t) => (t.value_out || 0) + Math.max(-vbOf(t), 0)

// The block's minter: for a PoS block the staker is in the COINSTAKE (the tx that
// consumes a real input) — never the empty coinbase. A cold-stake coinstake output is
// P2CS [staker(S), owner(D)]; the staker (S) is who minted the block.
const minter = computed(() => {
  const cs = (block.value?.tx || []).find((t) => txType(t) === 'coinstake')
  if (!cs) return null
  const out = (cs.vout || []).find((o) => (o.addresses || []).length && Number(o.value) > 0)
  const a = (out && out.addresses) || []
  if (a.length >= 2) return { staker: a[0], owner: a[1], cold: true }
  if (a.length === 1) return { staker: a[0], owner: null, cold: false }
  return null
})

const typeCounts = computed(() => {
  const c = { coinbase: 0, coinstake: 0, transparent: 0, shielding: 0, 'de-shielding': 0, shielded: 0 }
  for (const t of (block.value?.tx || [])) c[txType(t)]++
  return c
})

const donutOption = computed(() => {
  const p = palette()
  const c = typeCounts.value
  return {
    backgroundColor: 'transparent',
    tooltip: { ...baseOption(p).tooltip, trigger: 'item' },
    series: [{
      type: 'pie', radius: ['58%', '84%'], center: ['50%', '50%'],
      itemStyle: { borderColor: 'rgba(10,6,20,0.9)', borderWidth: 3 },
      label: { show: false }, labelLine: { show: false },
      data: [
        { name: 'coinstake', value: c.coinstake, itemStyle: { color: TYPE_COLOR.coinstake } },
        { name: 'coinbase', value: c.coinbase, itemStyle: { color: TYPE_COLOR.coinbase } },
        { name: 'transparent', value: c.transparent, itemStyle: { color: TYPE_COLOR.transparent } },
        { name: 'shielding', value: c.shielding, itemStyle: { color: TYPE_COLOR.shielding } },
        { name: 'de-shielding', value: c['de-shielding'], itemStyle: { color: TYPE_COLOR['de-shielding'] } },
        { name: 'shielded', value: c.shielded, itemStyle: { color: TYPE_COLOR.shielded } },
      ].filter((d) => d.value > 0),
    }],
  }
})

const totalOut = computed(() =>
  (block.value?.tx || []).reduce((s, t) => s + (t.value_out || 0), 0))
const totalFees = computed(() =>
  (block.value?.tx || []).reduce((s, t) => s + (t.fees || 0), 0))
</script>

<template>
  <div class="page">
    <div class="page-head">
      <div>
        <div class="eyebrow">CORE EXPLORER · /block-detail</div>
        <h1 class="page-title">Block <span class="neon-text">#{{ height }}</span></h1>
      </div>
      <div class="head-live" v-if="block">
        <RouterLink v-if="block.previousblockhash" :to="`/block/${Number(height) - 1}`" class="gbtn">‹ PREV</RouterLink>
        <RouterLink v-if="block.nextblockhash" :to="`/block/${Number(height) + 1}`" class="gbtn">NEXT ›</RouterLink>
      </div>
    </div>

    <div v-if="err" class="banner bad">{{ err }}</div>

    <template v-if="block">
      <div class="statgrid cols-5">
        <Stat k="REWARD" accent><template #v>{{ formatPiv(block.reward, { decimals: 2 }) }}</template><template #s>PIV minted</template></Stat>
        <Stat k="TRANSACTIONS" glow><template #v>{{ block.tx.length }}</template><template #s>{{ typeCounts.transparent }} payment</template></Stat>
        <Stat k="VALUE OUT"><template #v>{{ formatPiv(totalOut, { decimals: 2 }) }}</template><template #s>PIV moved</template></Stat>
        <Stat k="CONFIRMATIONS"><template #v>{{ compactNumber(block.confirmations) }}</template><template #s>{{ timeAgo(block.time) }}</template></Stat>
        <Stat k="DIFFICULTY"><template #v>{{ formatDifficulty(block.difficulty) }}</template><template #s>fees {{ formatPiv(totalFees, { decimals: 8 }) }}</template></Stat>
      </div>

      <div class="split s-37" style="margin-top: var(--space-4)">
        <HudPanel title="TX-TYPE DISTRIBUTION" id="coinstake · coinbase · transparent · ◈ shield flows" hero>
          <div class="donut-wrap">
            <EChart :option="donutOption" height="200px" aria-label="Transaction-type distribution: coinstake, coinbase, transparent, shielding, de-shielding, shielded" />
            <div class="donut-legend">
              <div class="dl" v-for="(v,k) in typeCounts" :key="k" v-show="v>0">
                <span class="dl-dot" :style="{ background: TYPE_COLOR[k] }"></span>
                <span class="mono">{{ k }}</span>
                <span class="mono strong dl-v">{{ v }}</span>
              </div>
            </div>
          </div>
        </HudPanel>

        <HudPanel title="BLOCK HEADER" id="/block">
          <dl class="kv">
            <template v-if="minter">
              <dt>{{ minter.cold ? 'Staker' : 'Staked by' }}</dt>
              <dd>
                <RouterLink :to="`/address/${minter.staker}`">{{ minter.staker }}</RouterLink>
                <span v-if="minter.cold" class="pill cyan mono" style="margin-left:6px">COLD-STAKE</span>
              </dd>
              <template v-if="minter.cold">
                <dt>Owner</dt>
                <dd><RouterLink :to="`/address/${minter.owner}`">{{ minter.owner }}</RouterLink></dd>
              </template>
            </template>
            <dt>Hash</dt><dd>{{ block.hash }}</dd>
            <dt>Time</dt><dd>{{ formatDateTime(block.time) }}</dd>
            <dt>Merkle root</dt><dd>{{ block.merkleroot }}</dd>
            <dt>Bits</dt><dd>{{ block.bits }} · v{{ block.version }}</dd>
            <dt>Size (approx)</dt><dd>{{ formatCount(block.size) }} B</dd>
            <dt>Previous</dt><dd>{{ truncateHash(block.previousblockhash, 18, 14) }}</dd>
            <dt>Next</dt><dd>{{ block.nextblockhash ? truncateHash(block.nextblockhash, 18, 14) : '— (tip)' }}</dd>
          </dl>
        </HudPanel>
      </div>

      <h2 class="section-title">Transactions ({{ block.tx.length }})</h2>
      <HudPanel v-for="t in block.tx" :key="t.txid" :title="`TX ${truncateHash(t.txid, 8, 6)}`" :id="`${t.vin.length} in · ${t.vout.length} out`" class="txp">
        <template #head>
          <span class="pill" :class="{ neon: txType(t)==='coinstake', warn: txType(t)==='coinbase', cyan: txType(t)==='transparent', pink: SHIELD_TYPES.includes(txType(t)) }">{{ SHIELD_TYPES.includes(txType(t)) ? '◈ ' + txType(t) : txType(t) }}</span>
          <span v-if="isShielded(t) && !shieldShape(t)" class="pill pink mono" style="margin-left:4px">SHIELDED</span>
          <RouterLink :to="`/tx/${t.txid}`" class="gbtn">OPEN ↗</RouterLink>
        </template>
        <div class="txflow">
          <div class="flow-col">
            <div class="eyebrow">INPUTS · sat → PIV</div>
            <div v-for="(vin, i) in t.vin" :key="i" class="flow-row">
              <span v-if="vinCold(t, vin)" class="mono" style="display:inline-flex;flex-direction:column;gap:1px;line-height:1.35">
                <span class="dim">{{ vinCold(t, vin)[1] }} <span class="pill neon mono" style="padding:0 4px">OWNER</span></span>
                <span class="dim">{{ vinCold(t, vin)[0] }} <span class="pill cyan mono" style="padding:0 4px">STAKER</span></span>
              </span>
              <span v-else class="dim mono">{{ vin.coinbase ? 'coinbase' : (vin.address || '—') }}</span>
              <span class="mono num">{{ vinCold(t, vin) ? (vinColdValueSat(t) != null ? formatSats(vinColdValueSat(t), { decimals: 4 }) : '—') : (vin.value != null ? formatSats(vin.value, { decimals: 4 }) : '—') }}</span>
            </div>
            <!-- de-shield: the pool is the input side; amount = value_balance (PIV float) -->
            <div v-if="shieldShape(t) === 'de-shielding' || shieldShape(t) === 'shielded'" class="flow-row shield-row">
              <span class="mono shield-glyph" title="sapling shield pool — counterparty private">◈ shield pool</span>
              <span class="mono num">{{ shieldShape(t) === 'shielded' ? '—' : formatPiv(t.sapling.value_balance, { decimals: 4 }) }}</span>
            </div>
          </div>
          <div class="flow-mid">
            <span class="flow-arrow">→</span>
            <div class="flow-agg mono">
              <span class="dim">in</span> {{ formatPiv(txIn(t), { decimals: 4 }) }}
              <span class="dim">out</span> {{ formatPiv(txOut(t), { decimals: 4 }) }}
              <span class="dim">fee</span> {{ formatPiv(t.fees, { decimals: 8 }) }}
            </div>
          </div>
          <div class="flow-col">
            <div class="eyebrow">OUTPUTS · sat → PIV</div>
            <div v-for="(vout, i) in t.vout" :key="i" class="flow-row">
              <span v-if="vout.addresses && vout.addresses.length >= 2" class="mono" style="display:inline-flex;flex-direction:column;gap:1px;line-height:1.35">
                <span class="dim">{{ vout.addresses[1] }} <span class="pill neon mono" style="padding:0 4px">OWNER</span></span>
                <span class="dim">{{ vout.addresses[0] }} <span class="pill cyan mono" style="padding:0 4px">STAKER</span></span>
              </span>
              <span v-else class="dim mono">{{ (vout.addresses && vout.addresses[0]) ? vout.addresses[0] : '—' }}</span>
              <span class="mono num strong">{{ formatSats(vout.value, { decimals: 4 }) }}</span>
            </div>
            <!-- shielding: the pool is the output side; amount = |value_balance| (PIV float) -->
            <div v-if="shieldShape(t) === 'shielding' || shieldShape(t) === 'shielded'" class="flow-row shield-row">
              <span class="mono shield-glyph" title="sapling shield pool — counterparty private">◈ shield pool</span>
              <span class="mono num strong">{{ shieldShape(t) === 'shielded' ? '—' : formatPiv(-t.sapling.value_balance, { decimals: 4 }) }}</span>
            </div>
          </div>
        </div>
      </HudPanel>
    </template>

    <div v-else-if="!err" class="sk" style="height: 200px"></div>
  </div>
</template>

<style scoped>
.head-live { display: flex; gap: 8px; margin-left: auto; }
.donut-wrap { display: grid; grid-template-columns: 1fr auto; align-items: center; gap: var(--space-3); }
.donut-legend { display: flex; flex-direction: column; gap: 8px; }
.dl { display: flex; align-items: center; gap: 8px; font-size: 12px; }
.dl-dot { width: 10px; height: 10px; border-radius: 2px; box-shadow: 0 0 6px currentColor; }
.dl-v { margin-left: auto; }
.txp { margin-bottom: var(--space-3); }
.txflow { display: grid; grid-template-columns: 1fr auto 1fr; gap: var(--space-4); align-items: center; }
.flow-col { display: flex; flex-direction: column; gap: 4px; }
.flow-row { display: flex; justify-content: space-between; gap: 12px; font-size: 12px; padding: 4px 8px; border-radius: 6px; background: rgba(150,90,220,0.05); }
.flow-row .num { color: var(--text); }
.flow-mid { display: flex; flex-direction: column; align-items: center; gap: 8px; }
.flow-arrow { color: var(--neon); font-size: 20px; text-shadow: var(--glow-sm); }
.flow-agg { font-size: 10.5px; text-align: center; line-height: 1.7; color: var(--text-muted); }
@media (max-width: 760px) {
  .txflow { grid-template-columns: 1fr; }
  .donut-wrap { grid-template-columns: 1fr; }
}
</style>
