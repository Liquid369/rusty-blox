<script setup>
/* =====================================================================
   MASTERNODE DETAIL — derived from one /mnlist row (no dedicated route).
   Keyed by collateral txhash (the stable unique id). Collateral outpoint
   links to /tx, payee links to /address.
   UNITS: no money fields; lastseen/lastpaid = unix sec, activetime =
   DURATION sec (formatDuration, not timeAgo).
   ===================================================================== */
import { ref, onMounted, computed, watch } from 'vue'
import { getMnList } from '../api/client.js'
import { timeAgo, formatDuration, formatDateTime, truncateHash } from '../lib/format.js'
import HudPanel from '../components/HudPanel.vue'
import Stat from '../components/Stat.vue'

const props = defineProps({ id: { type: String, required: true } })
const node = ref(null)
const notFound = ref(false)
const err = ref(null)
const loading = ref(true)

async function load() {
  node.value = null; notFound.value = false; err.value = null
  loading.value = true
  try {
    const list = await getMnList()
    const m = list.find((n) => n.txhash === props.id || n.addr === props.id || `${n.txhash}-${n.outidx}` === props.id)
    if (m) node.value = m
    else notFound.value = true
  } catch (e) {
    err.value = e.message || 'failed to load masternode'
  } finally {
    loading.value = false
  }
}
onMounted(load)
watch(() => props.id, load)

function statusCls(s) {
  if (s === 'ENABLED') return 'ok'
  if (s === 'PRE_ENABLED') return 'cyan'
  if (s === 'EXPIRED' || s === 'MISSING') return 'bad'
  return 'warn'
}
const paid = computed(() => node.value && node.value.lastpaid > 0)
</script>

<template>
  <div class="page">
    <div class="page-head">
      <div>
        <div class="eyebrow">MASTERNODE · derived from /mnlist</div>
        <h1 class="page-title">Masternode</h1>
      </div>
      <div class="head-live" v-if="node">
        <span class="pill" :class="statusCls(node.status)"><span class="dot" :class="node.status === 'ENABLED' ? 'live' : ''"></span>{{ node.status }}</span>
        <span class="pill cyan mono">RANK #{{ node.rank }}</span>
      </div>
    </div>

    <div v-if="notFound" class="banner bad">
      Masternode not found in the current roster snapshot. The list is unindexed and
      cached ~60s — it may have rotated. <RouterLink to="/masternodes">Back to roster ›</RouterLink>
    </div>

    <div v-else-if="err" class="banner bad">{{ err }}</div>

    <div v-else-if="loading" class="loading" style="margin-top: var(--space-4)">loading masternode telemetry…</div>

    <template v-else-if="node">
      <HudPanel>
        <div class="id-row">
          <span class="eyebrow">COLLATERAL OUTPOINT</span>
          <RouterLink :to="`/tx/${node.txhash}`" class="mono id-val">{{ node.txhash }}:{{ node.outidx }}</RouterLink>
        </div>
      </HudPanel>

      <div class="statgrid cols-4" style="margin-top: var(--space-4)">
        <Stat k="COLLATERAL" accent>
          <template #v>10,000<span class="unit">PIV</span></template>
          <template #s>fixed legacy MN bond</template>
        </Stat>
        <Stat k="UPTIME" glow>
          <template #v>{{ formatDuration(node.activetime) }}</template>
          <template #s>active since activation</template>
        </Stat>
        <Stat k="LAST PAID">
          <template #v>{{ paid ? timeAgo(node.lastpaid) : 'never' }}</template>
          <template #s>{{ paid ? formatDateTime(node.lastpaid) : 'awaiting first payout' }}</template>
        </Stat>
        <Stat k="LAST SEEN" live>
          <template #v>{{ timeAgo(node.lastseen) }}</template>
          <template #s>network heartbeat</template>
        </Stat>
      </div>

      <h2 class="section-title">Node identity</h2>
      <HudPanel title="DETAIL" :id="`legacy · v${node.version}`">
        <dl class="kv">
          <dt>Status</dt><dd><span class="pill" :class="statusCls(node.status)">{{ node.status }}</span></dd>
          <dt>Payment rank</dt><dd>#{{ node.rank }} <span class="dim">(0 = next to be paid)</span></dd>
          <dt>Type</dt><dd>{{ node.type }}</dd>
          <dt>Network</dt><dd><span class="pill" :class="node.network === 'onion' ? 'neon' : 'cyan'">{{ node.network }}</span></dd>
          <dt>Protocol</dt><dd>{{ node.version }}</dd>
          <dt>Payee address</dt><dd><RouterLink :to="`/address/${node.addr}`">{{ node.addr }}</RouterLink></dd>
          <dt>MN pubkey</dt><dd><RouterLink :to="`/address/${node.pubkey}`">{{ node.pubkey }}</RouterLink></dd>
          <dt>Collateral tx</dt><dd><RouterLink :to="`/tx/${node.txhash}`">{{ node.txhash }}</RouterLink></dd>
          <dt>Collateral vout</dt><dd>{{ node.outidx }}</dd>
          <dt>Active for</dt><dd>{{ formatDuration(node.activetime) }} <span class="dim">({{ Math.floor(node.activetime / 86400) }} days)</span></dd>
          <dt>Last seen</dt><dd>{{ formatDateTime(node.lastseen) }} <span class="dim">· {{ timeAgo(node.lastseen) }}</span></dd>
          <dt>Last paid</dt><dd>{{ paid ? `${formatDateTime(node.lastpaid)} · ${timeAgo(node.lastpaid)}` : 'never paid' }}</dd>
        </dl>
        <p class="note mono dim">
          No per-node payment history endpoint exists — open the
          <RouterLink :to="`/address/${node.addr}`">payee address</RouterLink> for the full payout ledger.
        </p>
      </HudPanel>
    </template>
  </div>
</template>

<style scoped>
.head-live { display: flex; align-items: center; gap: 10px; margin-left: auto; }
.unit { font-size: 0.5em; color: var(--text-dim); margin-left: 4px; }
.id-row { display: flex; flex-direction: column; gap: 6px; }
.id-val { font-size: 13px; color: var(--neon); word-break: break-all; }
.note { margin: var(--space-4) 0 0; font-size: 11px; padding-top: var(--space-3); border-top: 1px solid var(--hud-line); }
</style>
