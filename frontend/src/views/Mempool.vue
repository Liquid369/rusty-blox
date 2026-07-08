<script setup>
/* =====================================================================
   MEMPOOL — /mempool pending-tx snapshot. The node monitor uses a
   non-verbose getrawmempool, so per-tx size/fee and bytes/usage are
   ALWAYS null — we surface "—" with a note rather than fake numbers.
   The size-over-time sparkline is client-accumulated live while the page is
   open (there is no backend mempool-history endpoint), so it starts empty and
   fills as you watch.
   UNITS: no money fields available on this endpoint.
   ===================================================================== */
import { ref, onMounted, onUnmounted, computed } from 'vue'
import { getMempool } from '../api/client.js'
import { timeAgo, truncateHash, formatCount, formatDateTime } from '../lib/format.js'
import { baseOption, catAxis, valAxis, palette, areaFill, hexA, echarts } from '../lib/chart.js'
import EChart from '../components/EChart.vue'
import HudPanel from '../components/HudPanel.vue'
import Stat from '../components/Stat.vue'

const pool = ref(null)
const err = ref(null)
// Congestion series is accumulated client-side while the page is open: there is
// no backend mempool-history endpoint (the monitor reads non-verbose
// getrawmempool), so we poll the live snapshot every 10s and append {ts, txs}.
// It starts empty and fills as you watch — real session telemetry, not a mock.
const series = ref([])
let timer = null

async function poll() {
  // setInterval swallows rejections into unhandled-rejection noise and freezes
  // the page on "—"; catch so a backend hiccup keeps the last good snapshot and
  // only surfaces an error if we never managed a first load.
  try {
    const p = await getMempool()
    if (!p) return
    pool.value = p
    err.value = null
    series.value.push({ ts: Math.floor(Date.now() / 1000), txs: p.size })
    if (series.value.length > 120) series.value.shift() // keep ~20 min at 10s cadence
  } catch (e) {
    if (!pool.value) err.value = e.message || 'mempool unavailable'
  }
}

onMounted(() => {
  poll()
  timer = setInterval(poll, 10000)
})
onUnmounted(() => {
  if (timer) clearInterval(timer)
})

const txs = computed(() => pool.value?.transactions || [])
const oldest = computed(() => txs.value.length ? Math.min(...txs.value.map((t) => t.time)) : 0)

/* ---------- congestion sparkline (accumulated mempool size) ---------- */
const congestionOption = computed(() => {
  const p = palette()
  const rows = series.value
  if (!rows.length) return baseOption(p)
  return {
    ...baseOption(p),
    grid: { left: 36, right: 14, top: 16, bottom: 24, containLabel: true },
    tooltip: { ...baseOption(p).tooltip, formatter: (arr) => `${new Date(rows[arr[0].dataIndex].ts * 1000).toISOString().slice(11, 16)}Z<br/>${arr[0].value} pending` },
    xAxis: catAxis(rows.map((r) => r.ts), p, { axisLabel: { show: false } }),
    yAxis: valAxis(p, { axisLabel: { formatter: (v) => v } }),
    series: [{
      type: 'line', smooth: true, showSymbol: false, step: false,
      data: rows.map((r) => r.txs),
      lineStyle: { color: p.cyan, width: 2, shadowColor: hexA(p.cyan, 0.6), shadowBlur: 8 },
      areaStyle: { color: areaFill(echarts, p.cyan, 0.3, 0.01) },
    }],
  }
})
</script>

<template>
  <div class="page">
    <div class="page-head">
      <div>
        <div class="eyebrow">UNCONFIRMED · /mempool</div>
        <h1 class="page-title">Mempool</h1>
      </div>
      <div class="head-live">
        <span class="pill" :class="txs.length ? 'warn' : 'ok'"><span class="dot" :class="txs.length ? '' : 'live'"></span>{{ txs.length ? `${txs.length} PENDING` : 'IDLE' }}</span>
      </div>
    </div>

    <div v-if="err" class="banner bad" style="margin-top: var(--space-4)">{{ err }}</div>

    <!-- STAT BAND -->
    <div class="statgrid cols-4">
      <Stat k="PENDING TXS" accent live>
        <template #v>{{ pool ? formatCount(pool.size) : '—' }}</template>
        <template #s>awaiting confirmation</template>
      </Stat>
      <Stat k="POOL BYTES">
        <template #v>{{ pool && pool.bytes != null ? formatCount(pool.bytes) : '—' }}</template>
        <template #s>per-tx size is unavailable</template>
      </Stat>
      <Stat k="OLDEST PENDING">
        <template #v>{{ oldest ? timeAgo(oldest) : '—' }}</template>
        <template #s>first observed by the monitor</template>
      </Stat>
      <Stat k="USAGE">
        <template #v>—</template>
        <template #s>field not populated by node</template>
      </Stat>
    </div>

    <!-- CONGESTION -->
    <h2 class="section-title">Congestion — mempool size over time</h2>
    <HudPanel title="MEMPOOL CONGESTION" id="live 10s-poll telemetry · this session" hero>
      <EChart v-if="series.length" :option="congestionOption" height="160px" aria-label="Mempool size over time" />
      <p class="note mono dim">
        Fee/size histograms are not possible: the monitor reads non-verbose getrawmempool, so
        only txid + first-observed time exist per pending tx. This sparkline is client-accumulated.
      </p>
    </HudPanel>

    <!-- PENDING TABLE -->
    <h2 class="section-title">Pending transactions ({{ txs.length }})</h2>
    <HudPanel title="PENDING QUEUE" id="/mempool · sorted by seen time">
      <div v-if="!txs.length" class="loading">mempool is empty — no unconfirmed transactions.</div>
      <table v-else class="dtable">
        <thead>
          <tr><th>Txid</th><th class="num">Fee</th><th class="num">Size</th><th>Seen</th><th>Observed at</th></tr>
        </thead>
        <tbody>
          <tr v-for="t in txs" :key="t.txid">
            <td><RouterLink :to="`/tx/${t.txid}`">{{ truncateHash(t.txid, 12, 8) }}</RouterLink></td>
            <td class="num dim">{{ t.fee == null ? '—' : t.fee }}</td>
            <td class="num dim">{{ t.size == null ? '—' : t.size }}</td>
            <td class="dim">{{ timeAgo(t.time) }}</td>
            <td class="dim">{{ formatDateTime(t.time) }}</td>
          </tr>
        </tbody>
      </table>
    </HudPanel>
  </div>
</template>

<style scoped>
.head-live { display: flex; align-items: center; gap: 10px; margin-left: auto; }
.note { margin: var(--space-3) 0 0; font-size: 11px; padding-top: var(--space-3); border-top: 1px solid var(--hud-line); }
</style>
