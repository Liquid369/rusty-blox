<script setup>
/* =====================================================================
   MASTERNODE NETWORK — /mncount counts + /mnlist full roster.
   Centerpiece: network-distribution donut (ipv4/ipv6/onion partition
   `total`) + status spread + activetime longevity histogram, with a
   client-paginated roster table.
   UNITS: this group carries NO money fields (counts + unix/duration secs).
   ===================================================================== */
import { ref, onMounted, computed } from 'vue'
import { getMnCount, getMnList } from '../api/client.js'
import { timeAgo, formatDuration, formatCount, truncateHash } from '../lib/format.js'
import { echarts, baseOption, catAxis, valAxis, palette, hexA } from '../lib/chart.js'
import EChart from '../components/EChart.vue'
import HudPanel from '../components/HudPanel.vue'
import Stat from '../components/Stat.vue'

const counts = ref(null)
const nodes = ref([])
const page = ref(1)
const PER = 12
const loading = ref(true)
const error = ref(null)

onMounted(async () => {
  // /mnlist is ~686 KB and takes a few seconds; show telemetry-loading state
  // instead of an empty roster that reads as broken.
  try {
    counts.value = await getMnCount()
    nodes.value = await getMnList()
  } catch (e) {
    error.value = e.message || 'Failed to load masternode telemetry'
  } finally {
    loading.value = false
  }
})

const totalPages = computed(() => Math.max(1, Math.ceil(nodes.value.length / PER)))
const pageRows = computed(() => nodes.value.slice((page.value - 1) * PER, page.value * PER))

const statusCounts = computed(() => {
  const m = {}
  for (const n of nodes.value) m[n.status] = (m[n.status] || 0) + 1
  return m
})

function statusCls(s) {
  if (s === 'ENABLED') return 'ok'
  if (s === 'PRE_ENABLED') return 'cyan'
  if (s === 'EXPIRED' || s === 'MISSING') return 'bad'
  return 'warn'
}
function netCls(n) { return n === 'onion' ? 'neon' : 'cyan' }

/* ---------- network distribution donut (ipv4 / ipv6 / onion) ---------- */
const netOption = computed(() => {
  const p = palette()
  if (!counts.value) return baseOption(p)
  const c = counts.value
  return {
    backgroundColor: 'transparent',
    tooltip: { ...baseOption(p).tooltip, trigger: 'item', formatter: (d) => `${d.name}<br/>${formatCount(d.value)} nodes · ${d.percent}%` },
    series: [{
      type: 'pie', radius: ['58%', '84%'], center: ['50%', '50%'],
      itemStyle: { borderColor: 'rgba(10,6,20,0.9)', borderWidth: 3 },
      label: { show: false }, labelLine: { show: false },
      data: [
        { name: 'IPv4', value: c.ipv4, itemStyle: { color: p.neon } },
        { name: 'IPv6', value: c.ipv6, itemStyle: { color: p.cyan } },
        { name: 'Onion', value: c.onion, itemStyle: { color: p.amber } },
      ],
    }],
  }
})

/* ---------- activetime longevity histogram ---------- */
const AGE_BUCKETS = [
  { label: '<30d', lo: 0, hi: 30 }, { label: '1-3m', lo: 30, hi: 90 },
  { label: '3-6m', lo: 90, hi: 180 }, { label: '6-12m', lo: 180, hi: 365 },
  { label: '1-2y', lo: 365, hi: 730 }, { label: '>2y', lo: 730, hi: Infinity },
]
const ageOption = computed(() => {
  const p = palette()
  const counts2 = AGE_BUCKETS.map(() => 0)
  for (const n of nodes.value) {
    const days = n.activetime / 86400
    const idx = AGE_BUCKETS.findIndex((b) => days >= b.lo && days < b.hi)
    if (idx >= 0) counts2[idx]++
  }
  return {
    ...baseOption(p),
    grid: { left: 34, right: 14, top: 14, bottom: 26, containLabel: true },
    tooltip: { ...baseOption(p).tooltip, formatter: (arr) => `${arr[0].name}<br/>${arr[0].value} nodes` },
    xAxis: catAxis(AGE_BUCKETS.map((b) => b.label), p, { boundaryGap: true, axisLabel: { interval: 0 } }),
    yAxis: valAxis(p),
    series: [{
      type: 'bar', data: counts2, barWidth: '58%',
      itemStyle: { color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [{ offset: 0, color: p.cyan }, { offset: 1, color: hexA(p.cyan, 0.2) }]), borderRadius: [3, 3, 0, 0] },
    }],
  }
})

const queueHealth = computed(() => counts.value ? (counts.value.inqueue / counts.value.enabled) * 100 : 0)
</script>

<template>
  <div class="page">
    <div class="page-head">
      <div>
        <div class="eyebrow">PIVX MASTERNODES · /mncount + /mnlist</div>
        <h1 class="page-title">Masternode Network</h1>
      </div>
      <div class="head-live">
        <span class="pill ok"><span class="dot live"></span>{{ counts ? formatCount(counts.enabled) : '—' }} ENABLED</span>
        <span class="pill neon mono">10,000 PIV COLLATERAL</span>
      </div>
    </div>

    <HudPanel v-if="loading" title="MASTERNODE TELEMETRY" id="/mncount + /mnlist · ~686 KB">
      <div class="loading">loading roster telemetry…</div>
    </HudPanel>
    <div v-else-if="error" class="banner bad" style="margin-top: var(--space-4)">{{ error }}</div>
    <template v-else>
    <!-- COUNT TILES -->
    <div class="statgrid cols-4">
      <Stat k="TOTAL NODES" accent>
        <template #v>{{ counts ? formatCount(counts.total) : '—' }}</template>
        <template #s>{{ counts ? formatCount(counts.stable) : '—' }} stable</template>
      </Stat>
      <Stat k="ENABLED" glow live>
        <template #v>{{ counts ? formatCount(counts.enabled) : '—' }}</template>
        <template #s>eligible for payment</template>
      </Stat>
      <Stat k="IN QUEUE">
        <template #v>{{ counts ? formatCount(counts.inqueue) : '—' }}</template>
        <template #s>{{ queueHealth.toFixed(1) }}% of enabled</template>
      </Stat>
      <Stat k="NETWORK SPLIT">
        <template #v>{{ counts ? formatCount(counts.ipv4 + counts.ipv6) : '—' }}<span class="unit">clear</span></template>
        <template #s>{{ counts ? formatCount(counts.onion) : '—' }} via Tor</template>
      </Stat>
    </div>

    <!-- DISTRIBUTION CHARTS -->
    <h2 class="section-title">Network composition</h2>
    <div class="split s-2">
      <HudPanel title="NETWORK DISTRIBUTION" id="/mncount · ipv4 · ipv6 · onion" hero>
        <div class="net-row">
          <EChart v-if="counts" :option="netOption" height="190px" aria-label="Masternode network distribution across IPv4, IPv6, and onion" />
          <div class="net-legend" v-if="counts">
            <div class="nl"><span class="nl-dot" style="background:var(--neon)"></span><b>IPv4</b><span class="mono dim">{{ formatCount(counts.ipv4) }}</span></div>
            <div class="nl"><span class="nl-dot" style="background:var(--cyan)"></span><b>IPv6</b><span class="mono dim">{{ formatCount(counts.ipv6) }}</span></div>
            <div class="nl"><span class="nl-dot" style="background:var(--amber)"></span><b>Onion</b><span class="mono dim">{{ formatCount(counts.onion) }}</span></div>
            <div class="nl-status">
              <div class="eyebrow">STATUS SPREAD (sample)</div>
              <div v-for="(n, s) in statusCounts" :key="s" class="ss">
                <span class="pill" :class="statusCls(s)">{{ s }}</span>
                <span class="mono dim">{{ n }}</span>
              </div>
            </div>
          </div>
        </div>
      </HudPanel>
      <HudPanel title="NODE LONGEVITY" id="/mnlist · activetime histogram">
        <EChart v-if="nodes.length" :option="ageOption" height="220px" aria-label="Masternode longevity histogram by active time" />
        <div v-else class="sk" style="height:220px"></div>
      </HudPanel>
    </div>

    <!-- ROSTER -->
    <h2 class="section-title">Roster ({{ formatCount(nodes.length) }} of ~{{ counts ? formatCount(counts.total) : '2,111' }})</h2>
    <HudPanel title="MASTERNODE LIST" id="/mnlist · rank asc">
      <template #head><span class="pill cyan mono">PAGE {{ page }}/{{ totalPages }}</span></template>
      <div class="scroll">
        <table class="dtable">
          <thead>
            <tr>
              <th>Rank</th><th>Status</th><th>Type</th><th>Net</th>
              <th>Payee</th><th>Collateral</th><th>Last paid</th><th>Active for</th><th>Last seen</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="n in pageRows" :key="n.txhash">
              <td class="strong">#{{ n.rank }}</td>
              <td><span class="pill" :class="statusCls(n.status)">{{ n.status }}</span></td>
              <td class="dim">{{ n.type }}</td>
              <td><span class="pill" :class="netCls(n.network)">{{ n.network }}</span></td>
              <td><RouterLink :to="`/address/${n.addr}`">{{ truncateHash(n.addr, 8, 6) }}</RouterLink></td>
              <td><RouterLink :to="`/masternode/${n.txhash}`">{{ truncateHash(n.txhash, 6, 4) }}:{{ n.outidx }}</RouterLink></td>
              <td class="dim">{{ n.lastpaid ? timeAgo(n.lastpaid) : 'never' }}</td>
              <td class="dim">{{ formatDuration(n.activetime) }}</td>
              <td class="dim">{{ timeAgo(n.lastseen) }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <div class="pager">
        <button class="gbtn" :disabled="page <= 1" @click="page--">‹ PREV</button>
        <span class="mono dim">{{ page }} / {{ totalPages }}</span>
        <button class="gbtn" :disabled="page >= totalPages" @click="page++">NEXT ›</button>
      </div>
    </HudPanel>
    </template>
  </div>
</template>

<style scoped>
.head-live { display: flex; align-items: center; gap: 10px; margin-left: auto; }
.unit { font-size: 0.5em; color: var(--text-dim); margin-left: 3px; }
.net-row { display: grid; grid-template-columns: 1fr 1fr; gap: var(--space-3); align-items: center; }
.net-legend { display: flex; flex-direction: column; gap: 8px; }
.nl { display: flex; align-items: center; gap: 9px; font-size: 12px; }
.nl b { color: var(--text); width: 46px; }
.nl-dot { width: 10px; height: 10px; border-radius: 3px; box-shadow: var(--glow-xs); }
.nl-status { margin-top: 6px; padding-top: 8px; border-top: 1px solid var(--hud-line); display: flex; flex-direction: column; gap: 6px; }
.ss { display: flex; align-items: center; justify-content: space-between; font-size: 11px; }
.scroll { overflow-x: auto; }
.pager { display: flex; align-items: center; gap: 14px; justify-content: flex-end; margin-top: var(--space-3); }
.pager .gbtn:disabled { opacity: 0.4; cursor: not-allowed; }
@media (max-width: 760px) { .net-row { grid-template-columns: 1fr; } }
</style>
