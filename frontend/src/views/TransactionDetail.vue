<script setup>
/* =====================================================================
   TRANSACTION DETAIL — centerpiece value-flow Sankey (vin → vout) with
   spent/unspent state pills. UNITS: EVERY money field is a satoshi
   STRING → formatSats (BigInt-safe). spent: true/false/null(unknown).
   ===================================================================== */
import { ref, watch, onMounted, computed } from 'vue'
import { getTx, getBlockDetail } from '../api/client.js'
import { formatSats } from '../lib/money.js'
import { formatDateTime, truncateHash, formatCount, timeAgo } from '../lib/format.js'
import { baseOption, palette, hexA } from '../lib/chart.js'
import { isCoinstakeTx, isUnresolvedColdVin, coinstakeInputAddresses, coinstakeInputValueSat } from '../lib/coinstake.js'
import EChart from '../components/EChart.vue'
import HudPanel from '../components/HudPanel.vue'
import Stat from '../components/Stat.vue'

const props = defineProps({ txid: { type: String, required: true } })
const tx = ref(null)
const err = ref(null)
// Block reward (PIV float) for a cold-stake coinstake whose inputs the backend left
// unresolved; /tx carries no reward, so we fetch it from the owning block.
const blockReward = ref(null)

async function load() {
  err.value = null; tx.value = null; blockReward.value = null
  try {
    const t = await getTx(props.txid)
    tx.value = t
    if (isCoinstakeTx(t) && (t.vin || []).some(isUnresolvedColdVin) && t.blockHeight != null) {
      try { blockReward.value = (await getBlockDetail(t.blockHeight)).reward } catch { /* value falls back to — */ }
    }
  } catch (e) { err.value = e.message }
}
onMounted(load)
watch(() => props.txid, load)

// [staker(S), owner(D)] to show for an unresolved cold-stake input, else null.
const vinCold = (vin) => coinstakeInputAddresses(tx.value, vin)
// Consumed stake (satoshi) for the single input of a cold coinstake, else null.
const coldInputValueSat = computed(() => {
  const t = tx.value
  if (!t) return null
  const rewardSat = blockReward.value != null ? Math.round(blockReward.value * 1e8) : null
  // Pass the satoshi STRING (not Number(t.value)) so BigInt keeps full precision.
  return coinstakeInputValueSat(t.value, rewardSat, (t.vin || []).length)
})

function spentPill(v) {
  if (v === true) return { cls: 'bad', text: 'spent' }
  if (v === false) return { cls: 'ok', text: 'unspent' }
  return { cls: 'warn', text: 'unknown' }
}
// satoshi string -> PIV number (display only; never used for money text)
const sat2piv = (v) => parseFloat(formatSats(v, { decimals: 8, group: false })) || 0

/* ---------- value-flow Sankey ---------- */
const sankeyOption = computed(() => {
  const p = palette()
  if (!tx.value) return baseOption(p)
  const t = tx.value
  const nodes = []
  const links = []
  const seen = new Map()
  const node = (id, side) => {
    if (!seen.has(id)) {
      seen.set(id, true)
      nodes.push({ name: id, itemStyle: { color: side === 'in' ? p.cyan : p.neon, borderColor: 'rgba(7,4,13,0.6)' } })
    }
    return id
  }
  const TXC = '◇ TX'
  nodes.push({ name: TXC, itemStyle: { color: p.amber }, label: { color: p.amber, fontWeight: 700 } })

  // For a cold-stake (P2CS) input/output, addresses = [staker(S), owner(D)] — the
  // OWNER is the economic party in the flow, so label the node with the last address.
  t.vin.forEach((vin, i) => {
    // Recover the cold-staker for unresolved P2CS coinstake inputs (backend blanks them).
    const cold = vinCold(vin)
    const a = cold || vin.addresses || []
    const addr = a.length ? truncateHash(a[a.length - 1], 6, 5) : `coinbase#${i}`
    const v = cold ? (coldInputValueSat.value || 0) / 1e8 : sat2piv(vin.value)
    if (v > 0) links.push({ source: node(`in:${addr}`, 'in'), target: TXC, value: v })
  })
  t.vout.forEach((vout, i) => {
    const a = vout.addresses || []
    const addr = a.length ? truncateHash(a[a.length - 1], 6, 5) : `out#${i}`
    const v = sat2piv(vout.value)
    if (v > 0) links.push({ source: TXC, target: node(`out:${addr}`, 'out'), value: v })
  })

  return {
    backgroundColor: 'transparent',
    tooltip: { ...baseOption(p).tooltip, trigger: 'item', formatter: (d) => d.dataType === 'edge' ? `${d.value.toFixed(4)} PIV` : d.name.replace(/^(in|out):/, '') },
    series: [{
      type: 'sankey', left: 14, right: 104, top: 16, bottom: 16,
      nodeWidth: 14, nodeGap: 12,
      data: nodes, links,
      label: { color: p.text, fontFamily: 'monospace', fontSize: 10.5, overflow: 'none', formatter: (d) => d.name.replace(/^(in|out):/, '') },
      lineStyle: { color: 'gradient', opacity: 0.32, curveness: 0.5 },
      itemStyle: { borderWidth: 1 },
      emphasis: { focus: 'adjacency', lineStyle: { opacity: 0.55 } },
    }],
  }
})
</script>

<template>
  <div class="page">
    <div class="page-head">
      <div>
        <div class="eyebrow">CORE EXPLORER · /tx</div>
        <h1 class="page-title">Transaction</h1>
      </div>
    </div>

    <div v-if="err" class="banner bad">{{ err }}</div>

    <template v-if="tx">
      <HudPanel class="txid-panel">
        <div class="txid-row">
          <span class="pill mono" :class="tx.confirmations > 0 ? 'neon' : 'warn'"><span class="dot" :class="tx.confirmations > 0 ? 'neon' : 'warn'"></span>{{ tx.confirmations > 0 ? 'CONFIRMED' : 'UNCONFIRMED' }}</span>
          <span class="mono txid-val">{{ tx.txid }}</span>
        </div>
      </HudPanel>

      <div class="statgrid cols-4" style="margin-top: var(--space-4)">
        <Stat k="VALUE OUT" accent><template #v>{{ formatSats(tx.value, { decimals: 4 }) }}</template><template #s>PIV</template></Stat>
        <Stat k="VALUE IN"><template #v>{{ formatSats(coldInputValueSat != null ? coldInputValueSat : tx.valueIn, { decimals: 4 }) }}</template><template #s>PIV</template></Stat>
        <Stat k="FEES" glow><template #v>{{ formatSats(tx.fees) }}</template><template #s>PIV</template></Stat>
        <Stat k="CONFIRMATIONS" live><template #v>{{ formatCount(tx.confirmations) }}</template><template #s>{{ timeAgo(tx.blockTime) }}</template></Stat>
      </div>

      <h2 class="section-title">Value flow</h2>
      <HudPanel title="VIN → VOUT VALUE-FLOW" id="sankey · satoshi → PIV" hero>
        <template #head><span class="pill cyan mono">{{ tx.vin.length }} in</span><span class="pill neon mono">{{ tx.vout.length }} out</span></template>
        <EChart :option="sankeyOption" height="300px" aria-label="Transaction value flow from inputs to outputs" />
      </HudPanel>

      <div class="split s-2" style="margin-top: var(--space-4)">
        <HudPanel title="INPUTS" :id="`${tx.vin.length} vin`">
          <table class="dtable">
            <thead><tr><th>Address</th><th class="num">Value (PIV)</th></tr></thead>
            <tbody>
              <tr v-for="(vin, i) in tx.vin" :key="i">
                <td>
                  <template v-if="vinCold(vin)">
                    <div style="display:flex;align-items:center;gap:6px;margin:1px 0"><RouterLink :to="`/address/${vinCold(vin)[1]}`">{{ truncateHash(vinCold(vin)[1], 8, 6) }}</RouterLink><span class="pill neon mono">OWNER</span></div>
                    <div style="display:flex;align-items:center;gap:6px;margin:1px 0"><RouterLink :to="`/address/${vinCold(vin)[0]}`">{{ truncateHash(vinCold(vin)[0], 8, 6) }}</RouterLink><span class="pill cyan mono">STAKER</span></div>
                  </template>
                  <template v-else-if="vin.addresses && vin.addresses.length >= 2">
                    <div style="display:flex;align-items:center;gap:6px;margin:1px 0"><RouterLink :to="`/address/${vin.addresses[1]}`">{{ truncateHash(vin.addresses[1], 8, 6) }}</RouterLink><span class="pill neon mono">OWNER</span></div>
                    <div style="display:flex;align-items:center;gap:6px;margin:1px 0"><RouterLink :to="`/address/${vin.addresses[0]}`">{{ truncateHash(vin.addresses[0], 8, 6) }}</RouterLink><span class="pill cyan mono">STAKER</span></div>
                  </template>
                  <RouterLink v-else-if="vin.addresses && vin.addresses[0]" :to="`/address/${vin.addresses[0]}`">{{ truncateHash(vin.addresses[0], 10, 8) }}</RouterLink>
                  <span v-else class="dim">coinbase</span>
                </td>
                <td class="num strong">{{ vinCold(vin) ? (coldInputValueSat != null ? formatSats(coldInputValueSat, { decimals: 4 }) : '—') : (vin.value != null ? formatSats(vin.value, { decimals: 4 }) : '—') }}</td>
              </tr>
            </tbody>
          </table>
        </HudPanel>

        <HudPanel title="OUTPUTS" :id="`${tx.vout.length} vout`">
          <table class="dtable">
            <thead><tr><th>Address</th><th class="num">Value (PIV)</th><th>State</th></tr></thead>
            <tbody>
              <tr v-for="(vout, i) in tx.vout" :key="i">
                <td>
                  <template v-if="vout.addresses && vout.addresses.length >= 2">
                    <div style="display:flex;align-items:center;gap:6px;margin:1px 0"><RouterLink :to="`/address/${vout.addresses[1]}`">{{ truncateHash(vout.addresses[1], 8, 6) }}</RouterLink><span class="pill neon mono">OWNER</span></div>
                    <div style="display:flex;align-items:center;gap:6px;margin:1px 0"><RouterLink :to="`/address/${vout.addresses[0]}`">{{ truncateHash(vout.addresses[0], 8, 6) }}</RouterLink><span class="pill cyan mono">STAKER</span></div>
                  </template>
                  <RouterLink v-else-if="vout.addresses && vout.addresses[0]" :to="`/address/${vout.addresses[0]}`">{{ truncateHash(vout.addresses[0], 10, 8) }}</RouterLink>
                  <span v-else class="dim">—</span>
                </td>
                <td class="num strong">{{ formatSats(vout.value, { decimals: 4 }) }}</td>
                <td><span class="pill" :class="spentPill(vout.spent).cls">{{ spentPill(vout.spent).text }}</span></td>
              </tr>
            </tbody>
          </table>
        </HudPanel>
      </div>

      <h2 class="section-title">Context</h2>
      <HudPanel title="LEDGER CONTEXT" id="/tx metadata">
        <dl class="kv">
          <dt>Block height</dt><dd><RouterLink :to="`/block/${tx.blockHeight}`">#{{ tx.blockHeight }}</RouterLink></dd>
          <dt>Block hash</dt><dd>{{ truncateHash(tx.blockHash, 18, 14) }}</dd>
          <dt>Block time</dt><dd>{{ formatDateTime(tx.blockTime) }}</dd>
          <dt>Size</dt><dd>{{ formatCount(tx.size) }} B · vsize {{ formatCount(tx.vsize) }} B</dd>
          <dt>Version</dt><dd>{{ tx.version }} · locktime {{ tx.lockTime }}</dd>
        </dl>
      </HudPanel>
    </template>

    <div v-else-if="!err" class="sk" style="height: 200px"></div>
  </div>
</template>

<style scoped>
.txid-panel { margin-top: 0; }
.txid-row { display: flex; align-items: center; gap: 14px; flex-wrap: wrap; }
.txid-val { font-size: 12.5px; color: var(--text-muted); word-break: break-all; }
</style>
