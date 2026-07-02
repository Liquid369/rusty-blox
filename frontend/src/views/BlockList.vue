<script setup>
/* =====================================================================
   BLOCK LEDGER — paginated recent blocks from /block-stats + a tx-load
   throughput sparkline. block-stats returns count+1 rows, NEWEST-FIRST;
   `size` is header bytes only (not usable as block size).
   UNITS: block-stats carries NO money fields; difficulty is a derived
   decimal (formatDifficulty), never satoshis.
   ===================================================================== */
import { ref, onMounted, computed } from 'vue'
import { getRecentBlocks } from '../api/client.js'
import { timeAgo, truncateHash, formatCount, formatDifficulty, compactNumber } from '../lib/format.js'
import { echarts, baseOption, catAxis, valAxis, palette, areaFill, hexA } from '../lib/chart.js'
import EChart from '../components/EChart.vue'
import HudPanel from '../components/HudPanel.vue'
import Stat from '../components/Stat.vue'

const FETCH = 80
const blocks = ref([])           // newest-first (count+1 rows)
const page = ref(1)
const PER = 15
const loading = ref(true)
const error = ref(null)

onMounted(async () => {
  try { blocks.value = await getRecentBlocks(FETCH) }
  catch (e) { error.value = e.message || 'Failed to load block telemetry' }
  finally { loading.value = false }
})

const chrono = computed(() => [...blocks.value].reverse()) // L->R time axis
const totalPages = computed(() => Math.max(1, Math.ceil(blocks.value.length / PER)))
const pageRows = computed(() => blocks.value.slice((page.value - 1) * PER, page.value * PER))

// interval = seconds since the previous (older) block. blocks are newest-first
// so the older neighbour of row i is i+1.
function intervalFor(globalIdx) {
  const older = blocks.value[globalIdx + 1]
  const b = blocks.value[globalIdx]
  return older ? Math.max(0, b.time - older.time) : null
}

const avgTx = computed(() => blocks.value.length
  ? blocks.value.reduce((s, b) => s + b.tx_count, 0) / blocks.value.length : 0)
const avgInterval = computed(() => {
  const c = chrono.value, out = []
  for (let i = 1; i < c.length; i++) out.push(c[i].time - c[i - 1].time)
  return out.length ? out.reduce((s, v) => s + v, 0) / out.length : 0
})
const newest = computed(() => blocks.value[0] || null)

/* ---------- throughput sparkline: tx/block columns + difficulty line ---------- */
const sparkOption = computed(() => {
  const p = palette()
  const rows = chrono.value
  const base = baseOption(p)
  return {
    ...base,
    legend: { data: ['tx / block', 'difficulty'], top: 0, right: 6, textStyle: { color: p.text, fontFamily: 'monospace', fontSize: 10 }, itemWidth: 12, itemHeight: 8 },
    grid: { left: 40, right: 48, top: 26, bottom: 24, containLabel: true },
    xAxis: catAxis(rows.map((b) => b.height), p, { axisLabel: { interval: 12 } }),
    yAxis: [
      valAxis(p, { axisLabel: { color: hexA(p.cyan, 0.85) } }),
      valAxis(p, { scale: true, extra: { position: 'right', splitLine: { show: false } }, axisLabel: { formatter: (v) => compactNumber(v) } }),
    ],
    series: [
      { name: 'tx / block', type: 'bar', data: rows.map((b) => b.tx_count), barWidth: '52%', itemStyle: { color: hexA(p.cyan, 0.34), borderRadius: [2, 2, 0, 0] } },
      { name: 'difficulty', type: 'line', yAxisIndex: 1, smooth: true, showSymbol: false, data: rows.map((b) => b.difficulty), lineStyle: { color: p.neon, width: 2.2, shadowColor: hexA(p.neon, 0.7), shadowBlur: 10 }, areaStyle: { color: areaFill(echarts, p.neon, 0.28, 0.01) } },
    ],
  }
})
</script>

<template>
  <div class="page">
    <div class="page-head">
      <div>
        <div class="eyebrow">CHAIN LEDGER · /block-stats</div>
        <h1 class="page-title">Blocks</h1>
      </div>
      <div class="head-live">
        <span class="pill neon mono" v-if="newest"><span class="dot neon"></span>TIP #{{ formatCount(newest.height) }}</span>
      </div>
    </div>

    <HudPanel v-if="loading" title="CHAIN TELEMETRY" id="/block-stats">
      <div class="loading">loading block telemetry…</div>
    </HudPanel>
    <div v-else-if="error" class="banner bad" style="margin-top: var(--space-4)">{{ error }}</div>
    <template v-else>
    <!-- STAT BAND -->
    <div class="statgrid cols-4">
      <Stat k="LATEST HEIGHT" accent live>
        <template #v>{{ newest ? formatCount(newest.height) : '—' }}</template>
        <template #s>{{ newest ? timeAgo(newest.time) : '—' }}</template>
      </Stat>
      <Stat k="AVG BLOCK INTERVAL" glow>
        <template #v>{{ avgInterval.toFixed(1) }}<span class="unit">s</span></template>
        <template #s>~60s PoS target</template>
      </Stat>
      <Stat k="AVG TX / BLOCK">
        <template #v>{{ avgTx.toFixed(1) }}</template>
        <template #s>across {{ blocks.length }} blocks</template>
      </Stat>
      <Stat k="DIFFICULTY">
        <template #v>{{ newest ? formatDifficulty(newest.difficulty) : '—' }}</template>
        <template #s>derived from nBits</template>
      </Stat>
    </div>

    <!-- THROUGHPUT -->
    <h2 class="section-title">Throughput — tx load &amp; difficulty</h2>
    <HudPanel title="BLOCK THROUGHPUT" id="/block-stats · newest → oldest reversed" hero>
      <template #head><span class="pill cyan mono">{{ blocks.length }} BLK</span></template>
      <EChart v-if="blocks.length" :option="sparkOption" height="200px" aria-label="Block throughput: transactions per block and difficulty" />
      <div v-else class="sk" style="height:200px"></div>
    </HudPanel>

    <!-- TABLE -->
    <h2 class="section-title">Recent blocks</h2>
    <HudPanel title="BLOCK TABLE" id="height · age · txs · interval">
      <template #head><span class="pill mono">PAGE {{ page }}/{{ totalPages }}</span></template>
      <div class="scroll">
        <table class="dtable">
          <thead>
            <tr><th>Height</th><th>Age</th><th class="num">Txs</th><th class="num">Interval</th><th class="num">Difficulty</th><th class="num">Hdr bytes</th><th>Hash</th></tr>
          </thead>
          <tbody>
            <tr v-for="(b, i) in pageRows" :key="b.height">
              <td><RouterLink :to="`/block/${b.height}`" class="strong">#{{ formatCount(b.height) }}</RouterLink></td>
              <td class="dim">{{ timeAgo(b.time) }}</td>
              <td class="num">{{ b.tx_count }}</td>
              <td class="num dim">
                <template v-if="intervalFor((page - 1) * PER + i) !== null">{{ intervalFor((page - 1) * PER + i) }}s</template>
                <template v-else>—</template>
              </td>
              <td class="num dim">{{ formatDifficulty(b.difficulty) }}</td>
              <td class="num dim">{{ b.size }}</td>
              <td class="dim">{{ truncateHash(b.hash, 10, 6) }}</td>
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
.scroll { overflow-x: auto; }
.pager { display: flex; align-items: center; gap: 14px; justify-content: flex-end; margin-top: var(--space-3); }
.pager .gbtn:disabled { opacity: 0.4; cursor: not-allowed; }
</style>
