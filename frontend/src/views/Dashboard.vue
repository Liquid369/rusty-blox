<script setup>
/* =====================================================================
   MISSION CONTROL — live network heartbeat dashboard.
   Centerpiece: chain-tip telemetry + a hero "heartbeat" chart
   (recent-block difficulty area + tx-count columns + block-interval),
   a sync radial gauge, supply composition, and a live block feed.
   UNITS: supply = PIV (formatPiv); block-stats carry no money.
   ===================================================================== */
import { ref, onMounted, onBeforeUnmount, computed } from 'vue'
import { useChainStore } from '../store.js'
import { getRecentBlocks, getSupply, getHealth, getTransactions } from '../api/client.js'
import { compactNumber, formatCount, timeAgo, truncateHash, formatDifficulty } from '../lib/format.js'
import { echarts, baseOption, catAxis, valAxis, palette, areaFill, hexA } from '../lib/chart.js'
import EChart from '../components/EChart.vue'
import HudPanel from '../components/HudPanel.vue'
import Stat from '../components/Stat.vue'

const chain = useChainStore()
const blocks = ref([])          // newest-first (block-stats), count+1 rows
const supply = ref(null)
const health = ref(null)
const txSeries = ref([])
const now = ref(Math.floor(Date.now() / 1000))
let clk = null
let poll = null

// Re-fetch recent blocks so "since last block", the feed, and the charts track
// the chain tip instead of freezing at page-load. block-stats caches 60s
// server-side; a failed poll keeps the last good data.
async function loadBlocks() {
  try { blocks.value = await getRecentBlocks(40) } catch { /* keep last good data */ }
}

onMounted(() => {
  // Start the clock + block-feed poll FIRST: the old sequential awaits meant one
  // rejected fetch (supply/health/tx) threw before the timers were set, killing
  // the "since last block" counter and the live feed. allSettled isolates each load.
  clk = setInterval(() => { now.value = Math.floor(Date.now() / 1000) }, 1000)
  poll = setInterval(loadBlocks, 20000)
  Promise.allSettled([
    loadBlocks(),
    getSupply().then((v) => { supply.value = v }),
    getHealth().then((v) => { health.value = v }),
    getTransactions().then((v) => { txSeries.value = v }),
  ])
})
onBeforeUnmount(() => { clearInterval(clk); clearInterval(poll) })

// chronological order for L->R axes
const chrono = computed(() => [...blocks.value].reverse())
const newest = computed(() => blocks.value[0] || null)
// Prefer the live WS block time (no /block-stats cache lag); fall back to the
// newest polled block until the first WS event arrives.
const sinceLast = computed(() => {
  const t = chain.lastBlockAt || newest.value?.time || 0
  return t ? Math.max(0, now.value - t) : 0
})

// derived block intervals (consecutive time deltas; ~60s PoS target)
const intervals = computed(() => {
  const c = chrono.value
  const out = []
  for (let i = 1; i < c.length; i++) out.push(Math.max(0, c[i].time - c[i - 1].time))
  return out
})
const avgInterval = computed(() => {
  const a = intervals.value
  return a.length ? a.reduce((s, v) => s + v, 0) / a.length : 0
})
const todayTx = computed(() => txSeries.value.at(-1) || {})

const shieldPct = computed(() => supply.value ? supply.value.current.shield_adoption_percentage : 0)

/* ---------- HERO: heartbeat chart ---------- */
const heroOption = computed(() => {
  const p = palette()
  const rows = chrono.value
  const base = baseOption(p)
  return {
    ...base,
    grid: { left: 48, right: 48, top: 26, bottom: 28, containLabel: true },
    legend: {
      data: ['difficulty', 'tx / block'], top: 0, right: 8,
      textStyle: { color: p.text, fontFamily: 'monospace', fontSize: 10 },
      itemWidth: 12, itemHeight: 8,
    },
    tooltip: { ...base.tooltip },
    xAxis: catAxis(rows.map((b) => b.height), p, { axisLabel: { interval: 6 } }),
    yAxis: [
      valAxis(p, { scale: true, axisLabel: { formatter: (v) => compactNumber(v) } }),
      valAxis(p, { extra: { position: 'right', splitLine: { show: false } }, axisLabel: { color: hexA(p.cyan, 0.8) } }),
    ],
    series: [
      {
        name: 'tx / block', type: 'bar', yAxisIndex: 1,
        data: rows.map((b) => b.tx_count),
        barWidth: '46%',
        itemStyle: { color: hexA(p.cyan, 0.32), borderRadius: [2, 2, 0, 0] },
      },
      {
        name: 'difficulty', type: 'line', yAxisIndex: 0, smooth: true, showSymbol: false,
        data: rows.map((b) => b.difficulty),
        lineStyle: { color: p.neon, width: 2.4, shadowColor: hexA(p.neon, 0.8), shadowBlur: 12 },
        areaStyle: { color: areaFill(echarts, p.neon, 0.42, 0.01) },
        emphasis: { focus: 'series' },
      },
    ],
  }
})

/* ---------- block-interval health line ---------- */
const intervalOption = computed(() => {
  const p = palette()
  const base = baseOption(p)
  const data = intervals.value
  return {
    ...base,
    grid: { left: 36, right: 14, top: 18, bottom: 22, containLabel: true },
    xAxis: catAxis(data.map((_, i) => i), p, { axisLabel: { show: false } }),
    yAxis: valAxis(p, { scale: true, axisLabel: { formatter: '{value}s' } }),
    series: [
      {
        type: 'line', smooth: true, showSymbol: false, data,
        lineStyle: { color: p.cyan, width: 2 },
        areaStyle: { color: areaFill(echarts, p.cyan, 0.3, 0.01) },
        markLine: {
          silent: true, symbol: 'none',
          lineStyle: { color: hexA(p.amber, 0.7), type: 'dashed' },
          data: [{ yAxis: 60, label: { formatter: '60s target', color: p.amber, fontSize: 9, fontFamily: 'monospace' } }],
        },
      },
    ],
  }
})

/* ---------- sync radial gauge ---------- */
const gaugeOption = computed(() => {
  const p = palette()
  const pct = chain.syncPercentage || 0
  return {
    backgroundColor: 'transparent',
    series: [{
      type: 'gauge', startAngle: 220, endAngle: -40, radius: '96%', center: ['50%', '56%'],
      min: 0, max: 100, splitNumber: 4,
      progress: { show: true, width: 10, roundCap: true,
        itemStyle: { color: new echarts.graphic.LinearGradient(0, 0, 1, 1, [{ offset: 0, color: p.neon }, { offset: 1, color: p.cyan }]), shadowColor: hexA(p.neon, 0.7), shadowBlur: 14 } },
      axisLine: { lineStyle: { width: 10, color: [[1, hexA(p.neon, 0.1)]] } },
      axisTick: { show: false },
      splitLine: { length: 8, lineStyle: { color: hexA(p.neon, 0.3), width: 1 } },
      axisLabel: { distance: 14, color: p.axis, fontSize: 9, fontFamily: 'monospace' },
      pointer: { show: false },
      anchor: { show: false },
      title: { show: false },
      detail: {
        valueAnimation: true, offsetCenter: [0, '4%'],
        formatter: (v) => `${v.toFixed(1)}%`,
        color: p.neon, fontSize: 26, fontFamily: 'monospace', fontWeight: 700,
      },
      data: [{ value: pct }],
    }],
  }
})

/* ---------- supply composition donut ---------- */
const supplyOption = computed(() => {
  const p = palette()
  if (!supply.value) return baseOption(p)
  const cur = supply.value.current
  const t = parseFloat(cur.transparent_supply)
  const s = parseFloat(cur.shielded_supply)
  return {
    backgroundColor: 'transparent',
    tooltip: { ...baseOption(p).tooltip, trigger: 'item', formatter: (d) => `${d.name}<br/>${compactNumber(d.value)} PIV · ${d.percent}%` },
    series: [{
      type: 'pie', radius: ['62%', '86%'], center: ['50%', '50%'], avoidLabelOverlap: false,
      itemStyle: { borderColor: 'rgba(10,6,20,0.9)', borderWidth: 3 },
      label: { show: false }, labelLine: { show: false },
      data: [
        { name: 'Transparent', value: t, itemStyle: { color: p.neon } },
        { name: 'Shielded', value: s, itemStyle: { color: p.cyan } },
      ],
    }],
  }
})

function blockType(b, i) {
  return i === 0 ? { cls: 'neon', txt: 'TIP' } : { cls: '', txt: 'PoS' }
}
</script>

<template>
  <div class="page">
    <div class="page-head">
      <div>
        <div class="eyebrow">PIVX NETWORK · LIVE TELEMETRY</div>
        <h1 class="page-title">Mission Control</h1>
      </div>
      <div class="head-live">
        <span class="pill ok"><span class="dot live"></span>NETWORK ONLINE</span>
        <span class="pill neon mono">+{{ sinceLast }}s since last block</span>
      </div>
    </div>

    <!-- TELEMETRY STAT BAND -->
    <div class="statgrid cols-4">
      <Stat k="CHAIN TIP" live accent>
        <template #v>{{ formatCount(chain.height) }}</template>
        <template #s>network {{ formatCount(chain.networkHeight) }} · {{ chain.blocksBehind }} behind</template>
      </Stat>
      <Stat k="TOTAL SUPPLY" glow>
        <template #v>{{ supply ? compactNumber(supply.current.total_supply) : '—' }}</template>
        <template #s>PIV · {{ shieldPct.toFixed(3) }}% shielded</template>
      </Stat>
      <Stat k="TRANSACTIONS">
        <template #v>{{ health ? compactNumber(health.total_transactions) : '—' }}</template>
        <template #s>{{ health ? compactNumber(health.indexed_addresses) : '—' }} addresses indexed</template>
      </Stat>
      <Stat k="AVG BLOCK INTERVAL">
        <template #v>{{ avgInterval.toFixed(1) }}<span class="unit">s</span></template>
        <template #s>~60s PoS target · {{ todayTx.count ? compactNumber(todayTx.count) : '—' }} tx/day</template>
      </Stat>
      <Stat k="MARKET PRICE" glow>
        <template #v>{{ chain.price ? '$' + chain.price.usd.toFixed(4) : '—' }}</template>
        <template #s>{{ chain.price ? '€' + chain.price.eur.toFixed(4) + ' · ' + Math.round(chain.price.btc * 1e8).toLocaleString('en-US') + ' sats' : 'PIVX · USD / EUR / sats' }}</template>
      </Stat>
    </div>

    <!-- HERO HEARTBEAT + SIDE GAUGE -->
    <div class="split s-21 hero-row">
      <HudPanel title="NETWORK HEARTBEAT" id="/block-stats · difficulty × tx-load" hero>
        <template #head><span class="pill cyan mono">{{ blocks.length }} BLK</span></template>
        <EChart v-if="blocks.length" :option="heroOption" height="320px" />
        <div v-else class="sk" style="height:320px"></div>
        <div class="hero-sub">
          <div class="eyebrow">BLOCK-INTERVAL TAIL · CHAIN HEALTH</div>
          <EChart v-if="intervals.length" :option="intervalOption" height="120px" />
        </div>
      </HudPanel>

      <div class="stack">
        <HudPanel title="SYNC STATUS" id="/status" hero>
          <EChart :option="gaugeOption" height="180px" />
          <div class="gauge-cap">
            <span class="pill" :class="chain.synced ? 'ok' : 'warn'">
              <span class="dot" :class="chain.synced ? 'live' : ''"></span>{{ chain.synced ? 'SYNCED' : 'CATCHING UP' }}
            </span>
            <span class="mono dim">{{ formatCount(chain.blocksBehind) }} blocks behind</span>
          </div>
        </HudPanel>

        <HudPanel title="SUPPLY COMPOSITION" id="/analytics/supply">
          <div class="supply-row">
            <EChart v-if="supply" :option="supplyOption" height="150px" />
            <div class="supply-legend" v-if="supply">
              <div class="sl-item">
                <span class="sl-dot" style="background:var(--neon)"></span>
                <div><b>Transparent</b><span class="mono dim">{{ compactNumber(supply.current.transparent_supply) }} PIV</span></div>
              </div>
              <div class="sl-item">
                <span class="sl-dot" style="background:var(--cyan)"></span>
                <div><b>Shielded</b><span class="mono dim">{{ compactNumber(supply.current.shielded_supply) }} PIV</span></div>
              </div>
              <div class="sl-gauge">
                <div class="eyebrow">SHIELD ADOPTION</div>
                <div class="sl-bar"><i :style="{ width: Math.min(100, shieldPct * 12) + '%' }"></i></div>
                <span class="cyan-text mono">{{ shieldPct.toFixed(3) }}%</span>
              </div>
            </div>
          </div>
        </HudPanel>
      </div>
    </div>

    <!-- LIVE BLOCK FEED -->
    <h2 class="section-title">Incoming block feed</h2>
    <HudPanel title="RECENT BLOCKS" id="newest → oldest">
      <template #head><span class="pill neon mono"><span class="dot neon"></span>STREAMING</span></template>
      <div class="feed">
        <div v-if="!blocks.length" class="loading">awaiting telemetry…</div>
        <div v-for="(b, i) in blocks.slice(0, 14)" :key="b.height" class="feed-row" :class="{ fresh: i === 0 }">
          <span class="pill" :class="blockType(b, i).cls">{{ blockType(b, i).txt }}</span>
          <RouterLink :to="`/block/${b.height}`" class="feed-h mono">#{{ b.height }}</RouterLink>
          <span class="feed-age mono dim">{{ timeAgo(b.time) }}</span>
          <span class="feed-tx mono">{{ b.tx_count }} tx</span>
          <span class="feed-diff mono dim">Δ {{ formatDifficulty(b.difficulty) }}</span>
          <span class="feed-hash mono dim">{{ truncateHash(b.hash, 10, 6) }}</span>
        </div>
      </div>
    </HudPanel>
  </div>
</template>

<style scoped>
.head-live { display: flex; align-items: center; gap: 10px; margin-left: auto; }
.unit { font-size: 0.55em; color: var(--text-dim); margin-left: 2px; }
.hero-row { margin-top: var(--space-4); }
.hero-sub { margin-top: var(--space-3); padding-top: var(--space-3); border-top: 1px solid var(--hud-line); }
.gauge-cap { display: flex; align-items: center; justify-content: space-between; margin-top: 8px; font-size: 11px; }
.supply-row { display: grid; grid-template-columns: 150px 1fr; gap: var(--space-3); align-items: center; }
.supply-legend { display: flex; flex-direction: column; gap: 10px; }
.sl-item { display: flex; align-items: center; gap: 10px; }
.sl-dot { width: 10px; height: 10px; border-radius: 3px; box-shadow: var(--glow-xs); }
.sl-item div { display: flex; flex-direction: column; line-height: 1.25; }
.sl-item b { font-size: 12px; color: var(--text); }
.sl-item span { font-size: 11px; }
.sl-gauge { margin-top: 4px; }
.sl-bar { height: 6px; border-radius: 3px; background: rgba(70,230,208,0.12); overflow: hidden; margin: 5px 0; }
.sl-bar i { display: block; height: 100%; background: var(--cyan); box-shadow: var(--glow-cyan); }

.feed { display: flex; flex-direction: column; }
.feed-row {
  display: grid; grid-template-columns: 60px 90px 1fr 70px 130px 150px; align-items: center; gap: 12px;
  padding: 9px 8px; border-bottom: 1px solid var(--hud-line-2); font-size: 12.5px;
}
.feed-row:hover { background: rgba(196,107,255,0.05); }
.feed-row.fresh { background: linear-gradient(90deg, rgba(196,107,255,0.12), transparent); animation: freshin .6s ease; }
@keyframes freshin { from { background: rgba(196,107,255,0.3); } to { background: linear-gradient(90deg, rgba(196,107,255,0.12), transparent); } }
.feed-h { color: var(--neon); }
.feed-tx { color: var(--text-muted); text-align: right; }
.feed-hash { text-align: right; }
@media (max-width: 760px) {
  .feed-row { grid-template-columns: 50px 1fr auto; }
  .feed-age, .feed-diff, .feed-hash { display: none; }
  .supply-row { grid-template-columns: 1fr; }
}
</style>
