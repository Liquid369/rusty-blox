<script setup>
/* =====================================================================
   XPUB ACCOUNT — aggregate a whole HD account (/xpub?details=tokens).
   Account totals + balance-by-derived-address bar + receive/change split
   + per-address (token) table + the merged tx ledger. Handles the 503
   reindex gate like the address page.
   UNITS: all /xpub money fields are satoshi STRINGS (formatSats).
   `txs` is TRANSFER count, not unique txs.
   ===================================================================== */
import { ref, onMounted, computed, watch } from 'vue'
import { getXpub } from '../api/client.js'
import { formatSats } from '../lib/money.js'
import { timeAgo, truncateHash, formatCount, compactNumber, isUnconfirmedHeight } from '../lib/format.js'
import { echarts, baseOption, catAxis, valAxis, palette, hexA } from '../lib/chart.js'
import EChart from '../components/EChart.vue'
import HudPanel from '../components/HudPanel.vue'
import Stat from '../components/Stat.vue'

const props = defineProps({ xpub: { type: String, required: true } })
const info = ref(null)
const ledger = ref(null)
const reindexing = ref(false)
const err = ref(null)
const loading = ref(true)

async function load() {
  info.value = null; ledger.value = null; reindexing.value = false; err.value = null
  loading.value = true
  try {
    // The real /xpub returns tokens XOR transactions per detail mode (and never
    // txids), so the address breakdown and the merged ledger need separate calls.
    info.value = await getXpub(props.xpub, { details: 'tokens', pageSize: 25 })
    ledger.value = await getXpub(props.xpub, { details: 'txs', pageSize: 25 })
  } catch (e) {
    if (e.status === 503) reindexing.value = true
    else err.value = e.message
  } finally {
    loading.value = false
  }
}
onMounted(load)
watch(() => props.xpub, load)

const sat2piv = (v) => parseFloat(formatSats(v, { decimals: 8, group: false })) || 0
const tokens = computed(() => info.value?.tokens || [])
// path = m/44'/119'/account'/chain/index → split()[4] is the BIP44 chain field
// (0=receive, 1=change). [5] is the address index — using it mis-buckets every
// address whose index is 1 as "change" and every change addr at index 0 as "receive".
const chainOf = (path) => (path.split('/')[4] === '1' ? 'change' : 'receive')

/* ---------- balance by derived address ---------- */
const balOption = computed(() => {
  const p = palette()
  const t = tokens.value
  if (!t.length) return baseOption(p)
  return {
    ...baseOption(p),
    grid: { left: 52, right: 14, top: 14, bottom: 40, containLabel: true },
    tooltip: { ...baseOption(p).tooltip, formatter: (arr) => `${t[arr[0].dataIndex].path}<br/>${compactNumber(arr[0].value)} PIV` },
    xAxis: catAxis(t.map((_, i) => i), p, { boundaryGap: true, extra: { name: 'derivation index', nameLocation: 'middle', nameGap: 26, nameTextStyle: { color: p.axis, fontSize: 9, fontFamily: 'monospace' } } }),
    yAxis: valAxis(p, { scale: true, axisLabel: { formatter: (v) => compactNumber(v) } }),
    series: [{
      type: 'bar', data: t.map((tk) => sat2piv(tk.balance)), barWidth: '60%',
      itemStyle: { color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [{ offset: 0, color: p.neon }, { offset: 1, color: hexA(p.neon, 0.25) }]), borderRadius: [3, 3, 0, 0] },
    }],
  }
})

/* ---------- receive vs change split ---------- */
const splitOption = computed(() => {
  const p = palette()
  let recv = 0, chg = 0
  for (const t of tokens.value) {
    if (chainOf(t.path) === 'change') chg += sat2piv(t.balance)
    else recv += sat2piv(t.balance)
  }
  return {
    backgroundColor: 'transparent',
    tooltip: { ...baseOption(p).tooltip, trigger: 'item', formatter: (d) => `${d.name}<br/>${compactNumber(d.value)} PIV · ${d.percent}%` },
    series: [{
      type: 'pie', radius: ['56%', '82%'], center: ['50%', '50%'],
      itemStyle: { borderColor: 'rgba(10,6,20,0.9)', borderWidth: 3 },
      label: { show: false }, labelLine: { show: false },
      data: [
        { name: 'Receive', value: recv, itemStyle: { color: p.cyan } },
        { name: 'Change', value: chg, itemStyle: { color: p.neon } },
      ],
    }],
  }
})
</script>

<template>
  <div class="page">
    <div class="page-head">
      <div>
        <div class="eyebrow">HD ACCOUNT · /xpub</div>
        <h1 class="page-title">Extended Public Key</h1>
      </div>
      <div class="head-live">
        <span class="pill neon mono">XPUB</span>
        <span class="pill cyan mono" v-if="info">{{ info.usedTokens }} ADDRESSES</span>
      </div>
    </div>

    <HudPanel>
      <div class="xp-row"><span class="mono xp-val">{{ xpub }}</span></div>
    </HudPanel>

    <div v-if="reindexing" class="banner warn" style="margin-top: var(--space-4)">
      <span class="dot warn"></span> ADDRESS INDEX REINDEXING — explorer is catching up. Retry shortly.
    </div>
    <div v-else-if="err" class="banner bad" style="margin-top: var(--space-4)">{{ err }}</div>

    <HudPanel v-else-if="loading" title="ACCOUNT TELEMETRY" id="/xpub · tokens + txs" style="margin-top: var(--space-4)">
      <div class="loading">loading account telemetry…</div>
    </HudPanel>

    <template v-else-if="info">
      <div class="statgrid cols-4" style="margin-top: var(--space-4)">
        <Stat k="ACCOUNT BALANCE" accent>
          <template #v>{{ formatSats(info.balance, { decimals: 2 }) }}</template>
          <template #s>PIV across {{ info.usedTokens }} addresses</template>
        </Stat>
        <Stat k="TOTAL RECEIVED" glow>
          <template #v>{{ formatSats(info.totalReceived, { decimals: 2 }) }}</template>
          <template #s>PIV lifetime in</template>
        </Stat>
        <Stat k="TOTAL SENT">
          <template #v>{{ formatSats(info.totalSent, { decimals: 2 }) }}</template>
          <template #s>PIV lifetime out</template>
        </Stat>
        <Stat k="TRANSFERS">
          <template #v>{{ formatCount(info.txs) }}</template>
          <template #s>lifetime on-chain transfers</template>
        </Stat>
      </div>

      <h2 class="section-title">Fund distribution across the account</h2>
      <div class="split s-21">
        <HudPanel title="BALANCE BY DERIVED ADDRESS" id="/xpub tokens · address heatmap" hero>
          <EChart :option="balOption" height="240px" aria-label="Balance by derived address" />
        </HudPanel>
        <HudPanel title="RECEIVE / CHANGE" id="path chain index 0 vs 1">
          <EChart :option="splitOption" height="170px" aria-label="Receive versus change balance split" />
          <div class="flow-cap">
            <span class="mono"><i class="fd" style="background:var(--cyan)"></i>receive</span>
            <span class="mono"><i class="fd" style="background:var(--neon)"></i>change</span>
          </div>
        </HudPanel>
      </div>

      <h2 class="section-title">Derived addresses ({{ tokens.length }})</h2>
      <HudPanel title="ADDRESS BREAKDOWN" id="/xpub · per-address (tokens)">
        <div class="scroll">
          <table class="dtable">
            <thead><tr><th>Address</th><th>Path</th><th>Chain</th><th class="num">Transfers</th><th class="num">Balance (PIV)</th></tr></thead>
            <tbody>
              <tr v-for="t in tokens" :key="t.name">
                <td><RouterLink :to="`/address/${t.name}`">{{ truncateHash(t.name, 8, 6) }}</RouterLink></td>
                <td class="dim">{{ t.path }}</td>
                <td><span class="pill" :class="chainOf(t.path) === 'change' ? 'neon' : 'cyan'">{{ chainOf(t.path) }}</span></td>
                <td class="num dim">{{ formatCount(t.transfers) }}</td>
                <td class="num strong">{{ formatSats(t.balance, { decimals: 4 }) }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </HudPanel>

      <template v-if="ledger && ledger.transactions && ledger.transactions.length">
        <h2 class="section-title">Merged ledger (page {{ ledger.page || 1 }}/{{ ledger.totalPages || 1 }})</h2>
        <HudPanel title="TRANSACTIONS" id="/xpub · details=txs">
          <div class="scroll">
            <table class="dtable">
              <thead><tr><th>Txid</th><th class="num">Height</th><th>Age</th><th class="num">Value (PIV)</th><th class="num">Conf.</th></tr></thead>
              <tbody>
                <tr v-for="t in ledger.transactions" :key="t.txid">
                  <td><RouterLink :to="`/tx/${t.txid}`">{{ truncateHash(t.txid, 10, 8) }}</RouterLink></td>
                  <td class="num dim"><span v-if="isUnconfirmedHeight(t.blockHeight)" class="pill warn mono">UNCONFIRMED</span><span v-else>{{ formatCount(t.blockHeight) }}</span></td>
                  <td class="dim">{{ timeAgo(t.blockTime) }}</td>
                  <td class="num strong">{{ formatSats(t.value, { decimals: 4 }) }}</td>
                  <td class="num dim">{{ formatCount(t.confirmations) }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </HudPanel>
      </template>
    </template>
  </div>
</template>

<style scoped>
.head-live { display: flex; align-items: center; gap: 10px; margin-left: auto; }
.xp-row { display: flex; align-items: center; }
.xp-val { font-size: 12.5px; color: var(--text-muted); word-break: break-all; }
.flow-cap { display: flex; justify-content: center; gap: 18px; margin-top: 8px; font-size: 11px; color: var(--text-muted); }
.flow-cap .fd { display: inline-block; width: 8px; height: 8px; border-radius: 2px; margin-right: 5px; box-shadow: 0 0 5px currentColor; }
.scroll { overflow-x: auto; }
</style>
