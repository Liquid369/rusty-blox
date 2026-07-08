<script setup>
/* =====================================================================
   ANALYTICS DECK — the killer on-chain intelligence set.
   HODL-waves · tx-type composition + coin-days-destroyed · cold-staking
   adoption curve + churn · wealth concentration Lorenz + histogram +
   Gini/Nakamoto · staking economics · rich list · treasury.
   UNITS (footgun): richlist.balance + transactions.avg_value = SATOSHIS
   (formatSats); everything else analytics money = PIV (formatPiv).
   ===================================================================== */
import { ref, onMounted, computed } from 'vue'
import {
  getSupply, getTransactions, getStaking, getNetwork,
  getRichlist, getWealthDistribution, getHodl, getColdstaking, getTreasury,
} from '../api/client.js'
import { formatSats, formatPiv } from '../lib/money.js'
import { compactNumber, percent, truncateHash, formatCount } from '../lib/format.js'
import { echarts, baseOption, catAxis, valAxis, palette, areaFill, hexA } from '../lib/chart.js'
import EChart from '../components/EChart.vue'
import HudPanel from '../components/HudPanel.vue'
import Stat from '../components/Stat.vue'

const supply = ref(null)
const txs = ref([])
const staking = ref([])
const network = ref([])
const richlist = ref([])
const wealth = ref(null)
const hodl = ref(null)
const coldstaking = ref([])
const treasury = ref([])
const ready = ref(false)
const error = ref(null)

onMounted(async () => {
  // Nine /analytics series in parallel — render a loading state, not empty
  // panels, until the whole deck resolves.
  try {
    ;[supply.value, txs.value, staking.value, network.value, richlist.value,
      wealth.value, hodl.value, coldstaking.value, treasury.value] = await Promise.all([
      getSupply(), getTransactions(), getStaking(), getNetwork(),
      getRichlist(100), getWealthDistribution(), getHodl(), getColdstaking(), getTreasury(),
    ])
    ready.value = true
  } catch (e) {
    error.value = e.message || 'Failed to load analytics telemetry'
  }
})

const latestStake = computed(() => staking.value.at(-1) || {})
const latestNet = computed(() => network.value.at(-1) || {})
const netDelegated = computed(() => coldstaking.value.at(-1)?.net_cumulative)
const treasuryTotal = computed(() =>
  treasury.value.reduce((s, t) => s + parseFloat(t.total_paid || 0), 0))

/* ---------- HODL waves (cold -> hot horizontal stack) ---------- */
const HODL_COLORS = {
  '<1m': '#ff5470', '1-3m': '#ff8f5c', '3-6m': '#ffcf5c',
  '6-12m': '#7ad97a', '1-2y': '#46e6d0', '>2y': '#9d4ef0',
}
const hodlOption = computed(() => {
  const p = palette()
  if (!hodl.value) return baseOption(p)
  const bands = hodl.value.bands
  return {
    backgroundColor: 'transparent',
    grid: { left: 8, right: 8, top: 8, bottom: 8 },
    tooltip: { ...baseOption(p).tooltip, trigger: 'item', formatter: (s) => `${s.seriesName}<br/>${compactNumber(s.value)}% of unspent supply` },
    xAxis: { type: 'value', max: 100, show: false },
    yAxis: { type: 'category', data: ['HODL'], show: false },
    series: bands.map((b) => ({
      name: b.band, type: 'bar', stack: 'h', barWidth: 46,
      data: [b.percentage],
      label: { show: b.percentage > 5, position: 'inside', color: '#0b0716', fontFamily: 'monospace', fontWeight: 700, fontSize: 11, formatter: `${b.band}` },
      itemStyle: { color: HODL_COLORS[b.band] || p.neon, borderColor: 'rgba(7,4,13,0.5)', borderWidth: 1 },
    })),
  }
})

/* ---------- tx-type composition + coin-days-destroyed ---------- */
const txOption = computed(() => {
  const p = palette()
  const rows = txs.value
  const base = baseOption(p)
  const mk = (name, key, color) => ({
    name, type: 'line', stack: 'mix', smooth: true, showSymbol: false,
    areaStyle: { color: areaFill(echarts, color, 0.5, 0.05) },
    lineStyle: { width: 0 }, emphasis: { focus: 'series' },
    data: rows.map((r) => r[key]),
  })
  // cold-stake and shielded are OVERLAPPING subsets of the true partition
  // (payment + stake + coinbase = count), so stacking them inflates the total by
  // ~17%. Draw them as non-stacked dashed overlays ("of which N are cold-stake / shielded").
  const overlay = (name, key, color) => ({
    name, type: 'line', smooth: true, showSymbol: false,
    lineStyle: { color, width: 1.5, type: 'dashed' }, emphasis: { focus: 'series' },
    data: rows.map((r) => r[key]),
  })
  return {
    ...base,
    legend: { data: ['payment', 'stake', 'coinbase', 'cold-stake', 'shielded', 'coin-days destroyed'], top: 0, textStyle: { color: p.text, fontFamily: 'monospace', fontSize: 10 }, itemWidth: 12, itemHeight: 8 },
    grid: { left: 52, right: 56, top: 30, bottom: 28, containLabel: true },
    xAxis: catAxis(rows.map((r) => r.date), p, { axisLabel: { interval: 22 } }),
    yAxis: [
      valAxis(p, { axisLabel: { formatter: (v) => compactNumber(v) } }),
      valAxis(p, { extra: { position: 'right', splitLine: { show: false } }, axisLabel: { color: hexA(p.amber, 0.85), formatter: (v) => compactNumber(v) } }),
    ],
    series: [
      mk('payment', 'payment_count', p.cyan),
      mk('stake', 'stake_count', p.neon),
      mk('coinbase', 'coinbase_count', p.deep),
      overlay('cold-stake', 'coldstake_txs', '#7ad97a'),
      overlay('shielded', 'sapling_txs', '#ff5fd0'),
      {
        name: 'coin-days destroyed', type: 'line', yAxisIndex: 1, smooth: true, showSymbol: false,
        data: rows.map((r) => r.coin_days_destroyed),
        lineStyle: { color: p.amber, width: 2, shadowColor: hexA(p.amber, 0.6), shadowBlur: 8 },
      },
    ],
  }
})

/* ---------- address activity: active + new addresses ---------- */
const addrOption = computed(() => {
  const p = palette()
  const rows = txs.value
  const base = baseOption(p)
  return {
    ...base,
    legend: { data: ['active addresses', 'new addresses'], top: 0, textStyle: { color: p.text, fontFamily: 'monospace', fontSize: 10 }, itemWidth: 12, itemHeight: 8 },
    grid: { left: 50, right: 50, top: 30, bottom: 26, containLabel: true },
    xAxis: catAxis(rows.map((r) => r.date), p, { axisLabel: { interval: 22 } }),
    yAxis: [
      valAxis(p, { scale: true, axisLabel: { formatter: (v) => compactNumber(v) } }),
      valAxis(p, { extra: { position: 'right', splitLine: { show: false } }, axisLabel: { color: hexA(p.amber, 0.85), formatter: (v) => compactNumber(v) } }),
    ],
    series: [
      { name: 'active addresses', type: 'line', smooth: true, showSymbol: false, data: rows.map((r) => r.active_addresses), lineStyle: { color: p.cyan, width: 2.4, shadowColor: hexA(p.cyan, 0.6), shadowBlur: 8 }, areaStyle: { color: areaFill(echarts, p.cyan, 0.3, 0.01) } },
      { name: 'new addresses', type: 'bar', yAxisIndex: 1, data: rows.map((r) => r.new_addresses), itemStyle: { color: hexA(p.amber, 0.7) }, barWidth: '46%' },
    ],
  }
})

/* ---------- fee economics: avg fee (PIV) + fee/byte ---------- */
const feeOption = computed(() => {
  const p = palette()
  const rows = txs.value
  const base = baseOption(p)
  return {
    ...base,
    legend: { data: ['avg fee (PIV)', 'fee / byte (sat)'], top: 0, textStyle: { color: p.text, fontFamily: 'monospace', fontSize: 10 }, itemWidth: 12, itemHeight: 8 },
    grid: { left: 62, right: 52, top: 30, bottom: 26, containLabel: true },
    xAxis: catAxis(rows.map((r) => r.date), p, { axisLabel: { interval: 22 } }),
    yAxis: [
      valAxis(p, { scale: true, axisLabel: { formatter: (v) => Number(v).toFixed(4) } }),
      valAxis(p, { extra: { position: 'right', splitLine: { show: false } }, axisLabel: { color: hexA(p.rose, 0.85), formatter: (v) => Number(v).toFixed(0) } }),
    ],
    series: [
      { name: 'avg fee (PIV)', type: 'line', smooth: true, showSymbol: false, data: rows.map((r) => parseFloat(r.avg_fee)), lineStyle: { color: p.neon, width: 2.4, shadowColor: hexA(p.neon, 0.6), shadowBlur: 8 }, areaStyle: { color: areaFill(echarts, p.neon, 0.28, 0.01) } },
      { name: 'fee / byte (sat)', type: 'line', yAxisIndex: 1, smooth: true, showSymbol: false, data: rows.map((r) => r.avg_fee_per_byte), lineStyle: { color: hexA(p.rose, 0.9), width: 1.8 } },
    ],
  }
})

/* ---------- cold-staking adoption curve ---------- */
const coldOption = computed(() => {
  const p = palette()
  const rows = coldstaking.value
  const base = baseOption(p)
  return {
    ...base,
    grid: { left: 56, right: 16, top: 16, bottom: 26, containLabel: true },
    tooltip: { ...base.tooltip, formatter: (arr) => {
      const d = arr[0]
      return `${d.axisValue}<br/>net delegated ${compactNumber(d.value)} PIV`
    } },
    xAxis: catAxis(rows.map((r) => r.date), p, { axisLabel: { interval: 22 } }),
    yAxis: valAxis(p, { scale: true, axisLabel: { formatter: (v) => compactNumber(v) } }),
    series: [{
      name: 'net delegated', type: 'line', smooth: true, showSymbol: false,
      data: rows.map((r) => parseFloat(r.net_cumulative)),
      lineStyle: { color: p.neon, width: 2.6, shadowColor: hexA(p.neon, 0.8), shadowBlur: 14 },
      areaStyle: { color: areaFill(echarts, p.neon, 0.46, 0.02) },
    }],
  }
})

/* ---------- cold-staking churn (created up / spent down) ---------- */
const churnOption = computed(() => {
  const p = palette()
  const rows = coldstaking.value.slice(-45)
  const base = baseOption(p)
  return {
    ...base,
    grid: { left: 50, right: 12, top: 18, bottom: 22, containLabel: true },
    legend: { data: ['created', 'spent'], top: 0, right: 4, textStyle: { color: p.text, fontFamily: 'monospace', fontSize: 10 }, itemWidth: 10, itemHeight: 8 },
    xAxis: catAxis(rows.map((r) => r.date), p, { boundaryGap: true, axisLabel: { show: false } }),
    yAxis: valAxis(p, { axisLabel: { formatter: (v) => compactNumber(Math.abs(v)) } }),
    series: [
      { name: 'created', type: 'bar', stack: 'c', data: rows.map((r) => parseFloat(r.created)), itemStyle: { color: hexA(p.cyan, 0.8) } },
      { name: 'spent', type: 'bar', stack: 'c', data: rows.map((r) => -parseFloat(r.spent)), itemStyle: { color: hexA(p.rose, 0.7) } },
    ],
  }
})

/* ---------- wealth concentration: Pareto/Lorenz cumulative ---------- */
const lorenzOption = computed(() => {
  const p = palette()
  const rl = richlist.value
  if (!rl.length) return baseOption(p)
  let cum = 0
  const pts = rl.map((r) => { cum += r.percentage; return cum })
  const base = baseOption(p)
  const nak = wealth.value?.nakamoto_coefficient || 33
  return {
    ...base,
    grid: { left: 46, right: 16, top: 18, bottom: 30, containLabel: true },
    tooltip: { ...base.tooltip, formatter: (arr) => `Top ${arr[0].axisValue} holders<br/>${arr[0].value.toFixed(1)}% of supply` },
    xAxis: catAxis(rl.map((r) => r.rank), p, { axisLabel: { interval: 9 }, extra: { name: 'holder rank', nameLocation: 'middle', nameGap: 24, nameTextStyle: { color: p.axis, fontSize: 9, fontFamily: 'monospace' } } }),
    yAxis: valAxis(p, { extra: { max: 100 }, axisLabel: { formatter: '{value}%' } }),
    series: [{
      type: 'line', smooth: true, showSymbol: false, data: pts,
      lineStyle: { color: p.amber, width: 2.6, shadowColor: hexA(p.amber, 0.6), shadowBlur: 10 },
      areaStyle: { color: areaFill(echarts, p.amber, 0.3, 0.01) },
      markLine: {
        silent: true, symbol: 'none',
        data: [
          { xAxis: nak - 1, lineStyle: { color: p.neon, type: 'dashed' }, label: { formatter: `Nakamoto ${nak}`, color: p.neon, fontSize: 9, fontFamily: 'monospace', position: 'insideEndTop' } },
          { yAxis: 51, lineStyle: { color: hexA(p.cyan, 0.6), type: 'dotted' }, label: { formatter: '51%', color: p.cyan, fontSize: 9, fontFamily: 'monospace' } },
        ],
      },
    }],
  }
})

/* ---------- wealth histogram ---------- */
const histOption = computed(() => {
  const p = palette()
  if (!wealth.value) return baseOption(p)
  const h = wealth.value.histogram
  const base = baseOption(p)
  return {
    ...base,
    grid: { left: 64, right: 18, top: 10, bottom: 22, containLabel: true },
    tooltip: { ...base.tooltip, formatter: (arr) => `${arr[0].name} PIV<br/>${formatCount(arr[0].value)} addresses` },
    xAxis: valAxis(p, { extra: { type: 'log', min: 100 }, axisLabel: { formatter: (v) => compactNumber(v) } }),
    yAxis: { type: 'category', data: h.map((b) => b.range), axisLine: { lineStyle: { color: 'rgba(150,90,220,0.25)' } }, axisTick: { show: false }, axisLabel: { color: p.axis, fontSize: 10, fontFamily: 'monospace' } },
    series: [{
      type: 'bar', data: h.map((b) => b.count), barWidth: '62%',
      itemStyle: { color: new echarts.graphic.LinearGradient(0, 0, 1, 0, [{ offset: 0, color: hexA(p.neon, 0.5) }, { offset: 1, color: p.neon }]), borderRadius: [0, 3, 3, 0] },
      label: { show: true, position: 'right', color: p.text, fontFamily: 'monospace', fontSize: 10, formatter: (d) => `${Number(h[d.dataIndex].percentage).toFixed(2)}%` },
    }],
  }
})

/* ---------- staking economics ---------- */
const stakingOption = computed(() => {
  const p = palette()
  const rows = staking.value
  const base = baseOption(p)
  return {
    ...base,
    legend: { data: ['APY (staker)', 'gross yield', 'top-10 dominance'], top: 0, textStyle: { color: p.text, fontFamily: 'monospace', fontSize: 10 }, itemWidth: 12, itemHeight: 8 },
    grid: { left: 46, right: 46, top: 30, bottom: 26, containLabel: true },
    xAxis: catAxis(rows.map((r) => r.date), p, { axisLabel: { interval: 22 } }),
    yAxis: [
      valAxis(p, { scale: true, axisLabel: { formatter: '{value}%' } }),
      valAxis(p, { extra: { position: 'right', max: 100, splitLine: { show: false } }, axisLabel: { color: hexA(p.rose, 0.85), formatter: '{value}%' } }),
    ],
    series: [
      { name: 'APY (staker)', type: 'line', smooth: true, showSymbol: false, data: rows.map((r) => r.apy_estimate), lineStyle: { color: p.cyan, width: 2.4, shadowColor: hexA(p.cyan, 0.6), shadowBlur: 8 }, areaStyle: { color: areaFill(echarts, p.cyan, 0.22, 0.01) } },
      { name: 'gross yield', type: 'line', smooth: true, showSymbol: false, data: rows.map((r) => r.gross_yield_estimate), lineStyle: { color: p.neon, width: 2, type: 'dashed' } },
      { name: 'top-10 dominance', type: 'line', yAxisIndex: 1, smooth: true, showSymbol: false, data: rows.map((r) => r.top10_dominance), lineStyle: { color: hexA(p.rose, 0.9), width: 1.6 } },
    ],
  }
})

const richMax = computed(() => richlist.value.length ? parseFloat(formatSats(richlist.value[0].balance, { decimals: 0, group: false })) : 1)
function richWidth(bal) {
  const v = parseFloat(formatSats(bal, { decimals: 0, group: false }))
  return Math.max(2, (v / richMax.value) * 100)
}
</script>

<template>
  <div class="page">
    <div class="page-head">
      <div>
        <div class="eyebrow">ON-CHAIN INTELLIGENCE · PIVX</div>
        <h1 class="page-title">Analytics Deck</h1>
      </div>
      <div class="head-live">
        <span class="pill cyan mono">{{ txs.length }}-DAY SERIES</span>
        <span class="pill neon mono">RICH/WEALTH · SNAPSHOT</span>
      </div>
    </div>

    <HudPanel v-if="!ready && !error" title="ANALYTICS TELEMETRY" id="/analytics · nine series">
      <div class="loading">loading on-chain telemetry…</div>
    </HudPanel>
    <div v-else-if="error" class="banner bad" style="margin-top: var(--space-4)">{{ error }}</div>
    <template v-else>
    <!-- HEADLINE -->
    <div class="statgrid cols-5">
      <Stat k="TOTAL SUPPLY" accent>
        <template #v>{{ supply ? compactNumber(supply.current.total_supply) : '—' }}</template>
        <template #s>{{ supply ? percent(supply.current.shield_adoption_percentage, 3) : '—' }} shielded</template>
      </Stat>
      <Stat k="STAKER APY" glow>
        <template #v>{{ percent(latestStake.apy_estimate) }}</template>
        <template #s>gross {{ percent(latestStake.gross_yield_estimate) }}</template>
      </Stat>
      <Stat k="NET COLD-STAKED">
        <template #v>{{ netDelegated ? compactNumber(netDelegated) : '—' }}</template>
        <template #s>PIV delegated (P2CS)</template>
      </Stat>
      <Stat k="GINI" glow>
        <template #v>{{ wealth ? Number(wealth.gini).toFixed(4) : '—' }}</template>
        <template #s>Nakamoto {{ wealth ? wealth.nakamoto_coefficient : '—' }}</template>
      </Stat>
      <Stat k="DIFFICULTY">
        <template #v>{{ latestNet.difficulty || '—' }}</template>
        <template #s>orphan {{ percent(latestNet.orphan_rate) }} · {{ latestNet.blocks_per_day || '—' }} blk/day</template>
      </Stat>
    </div>

    <!-- HODL WAVES -->
    <h2 class="section-title">HODL waves — unspent value by coin age</h2>
    <HudPanel title="HODL WAVES" id="/analytics/hodl · cold → hot" hero>
      <template #head>
        <span class="pill neon mono" v-if="hodl">{{ percent(hodl.bands.at(-1).percentage, 1) }} held &gt;2y</span>
      </template>
      <EChart v-if="hodl" :option="hodlOption" height="92px" aria-label="HODL waves: unspent supply by coin age" />
      <div class="hodl-legend" v-if="hodl">
        <div v-for="b in hodl.bands" :key="b.band" class="hl">
          <span class="hl-dot" :style="{ background: HODL_COLORS[b.band] }"></span>
          <span class="hl-band mono">{{ b.band }}</span>
          <span class="hl-val mono">{{ percent(b.percentage, 1) }}</span>
          <span class="hl-piv mono dim">{{ formatPiv(b.value, { decimals: 0 }) }} PIV</span>
        </div>
      </div>
    </HudPanel>

    <!-- TX COMPOSITION + COLD STAKING -->
    <h2 class="section-title">Network activity &amp; delegation</h2>
    <div class="split s-2">
      <HudPanel title="TX-TYPE COMPOSITION" id="/analytics/transactions + CDD">
        <EChart v-if="txs.length" :option="txOption" height="300px" aria-label="Transaction-type composition and coin-days destroyed over 120 days" />
        <div v-else class="sk" style="height:300px"></div>
      </HudPanel>
      <HudPanel title="COLD-STAKING ADOPTION" id="/analytics/coldstaking · net_cumulative">
        <EChart v-if="coldstaking.length" :option="coldOption" height="180px" aria-label="Cold-staking net delegated adoption curve" />
        <div class="eyebrow churn-cap">DAILY DELEGATION CHURN · CREATED ▲ / SPENT ▼</div>
        <EChart v-if="coldstaking.length" :option="churnOption" height="110px" aria-label="Daily cold-staking delegation churn, created versus spent" />
      </HudPanel>
    </div>

    <!-- ADDRESS ACTIVITY + FEES -->
    <h2 class="section-title">Address activity &amp; fee economics</h2>
    <div class="split s-2">
      <HudPanel title="ADDRESS ACTIVITY" id="/analytics/transactions · active + new">
        <EChart v-if="txs.length" :option="addrOption" height="260px" aria-label="Active and new addresses per day" />
        <div v-else class="sk" style="height:260px"></div>
      </HudPanel>
      <HudPanel title="FEE ECONOMICS" id="/analytics/transactions · avg fee + fee/byte">
        <EChart v-if="txs.length" :option="feeOption" height="260px" aria-label="Average fee in PIV and fee per byte over time" />
        <div v-else class="sk" style="height:260px"></div>
      </HudPanel>
    </div>

    <!-- WEALTH -->
    <h2 class="section-title">Wealth concentration</h2>
    <div class="split s-2">
      <HudPanel title="HOLDER CONCENTRATION CURVE" id="cumulative % of supply by rank" hero>
        <template #head>
          <span class="pill warn mono" v-if="wealth">TOP-100 = {{ percent(wealth.top_100, 1) }}</span>
        </template>
        <EChart v-if="richlist.length" :option="lorenzOption" height="280px" aria-label="Wealth concentration curve: cumulative share of supply by holder rank" />
      </HudPanel>
      <HudPanel title="BALANCE DISTRIBUTION" id="/analytics/wealth-distribution · log scale">
        <EChart v-if="wealth" :option="histOption" height="280px" aria-label="Balance distribution histogram on a log scale" />
      </HudPanel>
    </div>

    <!-- STAKING -->
    <h2 class="section-title">Staking economics</h2>
    <HudPanel title="YIELD &amp; STAKER CONCENTRATION" id="/analytics/staking">
      <template #head>
        <span class="pill cyan mono">participation {{ percent(latestStake.participation_rate, 1) }}</span>
        <span class="pill mono">{{ latestStake.active_stakers || '—' }} stakers</span>
      </template>
      <EChart v-if="staking.length" :option="stakingOption" height="260px" aria-label="Staking yield and staker concentration over time" />
    </HudPanel>

    <!-- RICH LIST + TREASURY -->
    <h2 class="section-title">Holders &amp; treasury</h2>
    <div class="split s-2">
      <HudPanel title="RICH LIST" id="/analytics/richlist · balance = sats">
        <div class="scroll">
          <table class="dtable">
            <thead><tr><th>#</th><th>Address</th><th class="num">Balance (PIV)</th><th>Share</th><th class="num">Txs</th></tr></thead>
            <tbody>
              <tr v-for="r in richlist.slice(0, 16)" :key="r.rank">
                <td class="strong">{{ r.rank }}</td>
                <td><RouterLink :to="`/address/${r.address}`">{{ truncateHash(r.address, 8, 6) }}</RouterLink></td>
                <td class="num strong">{{ formatSats(r.balance, { decimals: 0 }) }}</td>
                <td>
                  <div class="cell-bar">
                    <div class="minibar"><i :style="{ width: richWidth(r.balance) + '%' }"></i></div>
                    <span class="mono dim">{{ percent(r.percentage, 2) }}</span>
                  </div>
                </td>
                <td class="num dim">{{ formatCount(r.txCount) }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </HudPanel>

      <HudPanel title="TREASURY PAYOUTS" id="/analytics/treasury · PIV">
        <template #head><span class="pill neon mono">Σ {{ compactNumber(treasuryTotal) }} PIV</span></template>
        <div class="scroll">
          <table class="dtable">
            <thead><tr><th>Height</th><th>Date</th><th class="num">Paid (PIV)</th><th class="num">Outputs</th></tr></thead>
            <tbody>
              <tr v-for="t in treasury.slice(-16).reverse()" :key="t.height">
                <td><RouterLink :to="`/block/${t.height}`">{{ t.height }}</RouterLink></td>
                <td class="dim">{{ t.date }}</td>
                <td class="num strong">{{ formatPiv(t.total_paid, { decimals: 2 }) }}</td>
                <td class="num dim">{{ t.n_outputs }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </HudPanel>
    </div>
    </template>
  </div>
</template>

<style scoped>
.head-live { display: flex; align-items: center; gap: 10px; margin-left: auto; }
.hodl-legend { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 10px; margin-top: var(--space-3); padding-top: var(--space-3); border-top: 1px solid var(--hud-line); }
.hl { display: flex; align-items: center; gap: 7px; font-size: 11px; }
.hl-dot { width: 9px; height: 9px; border-radius: 2px; box-shadow: 0 0 6px currentColor; }
.hl-band { color: var(--text-muted); width: 40px; }
.hl-val { color: var(--text); font-weight: 700; }
.hl-piv { margin-left: auto; }
.churn-cap { margin: var(--space-3) 0 4px; padding-top: var(--space-3); border-top: 1px solid var(--hud-line); }
.scroll { overflow-x: auto; }
.cell-bar { display: flex; align-items: center; gap: 8px; min-width: 120px; }
.cell-bar .minibar { flex: 1; }
.cell-bar span { font-size: 10.5px; white-space: nowrap; }
</style>
