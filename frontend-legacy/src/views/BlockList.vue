<template>
  <AppLayout>
    <div class="block-list-page">
      <div class="page-header">
        <h1>Blocks</h1>
        <p class="page-subtitle">Browse all blocks in the PIVX blockchain</p>
      </div>

      <!-- Summary band -->
      <div class="summary-band">
        <div class="summary-item">
          <span class="summary-label">Chain Tip</span>
          <span class="summary-value summary-accent">
            {{ tip !== null ? `#${formatNumber(tip)}` : '—' }}
          </span>
        </div>
        <div class="summary-item">
          <span class="summary-label">Avg Interval (page)</span>
          <span class="summary-value">{{ avgInterval }}</span>
        </div>
        <div class="summary-item">
          <span class="summary-label">Txs on Page</span>
          <span class="summary-value">{{ blocks.length ? formatNumber(pageTxTotal) : '—' }}</span>
        </div>
        <div class="summary-item">
          <span class="summary-label">Avg Size (page)</span>
          <span class="summary-value">{{ avgSizeKb }}</span>
        </div>
      </div>

      <!-- Error state (only when nothing could be loaded) -->
      <div v-if="error && blocks.length === 0 && !loading" class="error-container">
        <EmptyState
          icon="alert-triangle"
          title="Failed to Load Blocks"
          :message="error"
        >
          <template #action>
            <Button @click="retry">Try Again</Button>
          </template>
        </EmptyState>
      </div>

      <!-- Block table -->
      <div v-else class="table-card">
        <div class="table-scroll">
          <table class="block-table">
            <thead>
              <tr>
                <th class="th-left">Height</th>
                <th class="th-left">Age</th>
                <th class="th-right">Txs</th>
                <th class="th-left">Composition</th>
                <th class="th-right">Value Out (PIV)</th>
                <th class="th-right">Fees (PIV)</th>
                <th class="th-right">Reward (PIV)</th>
                <th class="th-right">Size (KB)</th>
                <th class="th-right">Difficulty</th>
                <th class="th-left">Staker</th>
              </tr>
            </thead>

            <!-- Loading skeleton rows -->
            <tbody v-if="loading">
              <tr v-for="i in SKELETON_ROWS" :key="`sk-${i}`" class="skeleton-row">
                <td v-for="col in 10" :key="col" class="skeleton-cell">
                  <SkeletonLoader variant="text" :width="col === 4 || col === 10 ? '90%' : '70%'" />
                </td>
              </tr>
            </tbody>

            <!-- Empty state -->
            <tbody v-else-if="blocks.length === 0">
              <tr>
                <td colspan="10" class="empty-cell">No blocks found for this page</td>
              </tr>
            </tbody>

            <!-- Rows (entrance animation when new blocks are prepended) -->
            <TransitionGroup v-else tag="tbody" name="row-enter">
              <BlockRow
                v-for="block in blocks"
                :key="block.height"
                :block="block"
                :tick="tick"
              />
            </TransitionGroup>
          </table>
        </div>
      </div>

      <!-- Pager + jump-to-height -->
      <div class="table-controls">
        <div class="pager">
          <Button
            variant="ghost"
            size="sm"
            :disabled="currentPage <= 1 || loading"
            @click="goToPage(currentPage - 1)"
          >
            ← Prev
          </Button>
          <span class="pager-info">
            Page {{ formatNumber(currentPage) }} of {{ formatNumber(totalPages) }}
            <span v-if="pageRange" class="pager-range">· {{ pageRange }}</span>
          </span>
          <Button
            variant="ghost"
            size="sm"
            :disabled="currentPage >= totalPages || loading"
            @click="goToPage(currentPage + 1)"
          >
            Next →
          </Button>
        </div>

        <form class="jump-form" @submit.prevent="jumpToHeight">
          <input
            v-model="jumpInput"
            class="jump-input"
            inputmode="numeric"
            pattern="[0-9]*"
            placeholder="Jump to height…"
            aria-label="Jump to block height"
          />
          <Button size="sm" variant="secondary" @click="jumpToHeight">Go</Button>
        </form>
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import api from '@/services/api'
import { formatNumber } from '@/utils/formatters'
import { TX_TYPES } from '@/utils/constants'
import { detectTransactionType, getAddressRoles, toSats } from '@/utils/transactionHelpers'
import AppLayout from '@/components/layout/AppLayout.vue'
import BlockRow from '@/components/blocks/BlockRow.vue'
import EmptyState from '@/components/common/EmptyState.vue'
import Button from '@/components/common/Button.vue'
import SkeletonLoader from '@/components/common/SkeletonLoader.vue'

const PAGE_SIZE = 25
const SKELETON_ROWS = 12
const STATUS_POLL_MS = 15000
const AGE_TICK_MS = 5000

const router = useRouter()

const tip = ref(null)          // newest known chain height (poll-updated)
const anchorTip = ref(null)    // tip the current page window is anchored to
const currentPage = ref(1)
const blocks = ref([])
const loading = ref(true)
const error = ref('')
const jumpInput = ref('')
const tick = ref(0)

let alive = true
let statusTimer = null
let tickTimer = null
let polling = false

// ---------------------------------------------------------------------------
// Mapping: /api/v2/block-detail -> row model
// ---------------------------------------------------------------------------

/**
 * The shared classifier was written against the /api/v2/tx shape where output
 * values are satoshi strings; block-detail returns satoshi numbers. Stringify
 * values (and attach blockHeight for budget detection) before classifying.
 */
const normalizeForClassifier = (tx, blockHeight) => ({
  ...tx,
  blockHeight,
  vout: Array.isArray(tx?.vout)
    ? tx.vout.map(v => ({ ...v, value: v?.value == null ? '0' : String(v.value) }))
    : []
})

const mapBlockDetail = (data) => {
  const txs = Array.isArray(data.tx) ? data.tx : []
  const typeCounts = { coinstake: 0, transparent: 0, shield: 0, coldstake: 0 }
  let totalSats = 0
  let feesPiv = 0
  let stakers = []

  // Block subsidy is authoritative from the backend's emission schedule
  // (deterministic by height) — never re-derived from outputs−inputs, which
  // breaks on zerocoin stakes whose inputs have no resolvable prevout value.
  // data.reward is PIV; the row formatter expects satoshis.
  const rewardSats = data.reward != null ? Math.round(data.reward * 1e8) : null

  for (const tx of txs) {
    const vouts = Array.isArray(tx?.vout) ? tx.vout : []
    const outSats = vouts.reduce((sum, v) => sum + toSats(v?.value), 0)
    totalSats += outSats

    // Fees are reported per-tx in PIV by the backend (0 for coinstake/coinbase)
    feesPiv += Number(tx?.fees) || 0

    const type = detectTransactionType(normalizeForClassifier(tx, data.height))

    if (type === TX_TYPES.COINSTAKE || type === TX_TYPES.BUDGET) {
      typeCounts.coinstake++

      // Staker = first value-bearing output; P2CS carries [staker, owner] roles
      const stakeOut = vouts.find(v => Array.isArray(v?.addresses) && v.addresses.length > 0)
      if (stakeOut) stakers = getAddressRoles(stakeOut).slice(0, 2)
    } else if (type === TX_TYPES.SAPLING) {
      typeCounts.shield++
    } else if (type === TX_TYPES.COLDSTAKE) {
      typeCounts.coldstake++
    } else if (type === TX_TYPES.COINBASE) {
      if (outSats > 0) typeCounts.coinstake++
    } else {
      typeCounts.transparent++
    }
  }

  return {
    height: data.height,
    hash: data.hash,
    time: data.time,
    txCount: txs.length,
    size: data.size || 0,
    difficulty: data.difficulty || 0,
    totalSats,
    feesSats: Math.round(feesPiv * 1e8),
    rewardSats,
    typeCounts,
    stakers
  }
}

// ---------------------------------------------------------------------------
// Fetching
// ---------------------------------------------------------------------------

const fetchTip = async () => {
  const response = await api.get('/api/v2/status')
  const data = response.data || {}
  const height = data.height ?? data.network_height
  if (!Number.isFinite(height)) throw new Error('Chain status unavailable')
  return height
}

/** Fetch block details for a set of heights in parallel; returns rows sorted desc. */
const fetchBlockRows = async (heights) => {
  const results = await Promise.allSettled(
    heights.map(h => api.get(`/api/v2/block-detail/${h}`))
  )
  const rows = []
  results.forEach(res => {
    if (res.status === 'fulfilled' && res.value?.data?.height !== undefined) {
      rows.push(mapBlockDetail(res.value.data))
    }
  })
  rows.sort((a, b) => b.height - a.height)
  return rows
}

const loadPage = async (page) => {
  loading.value = true
  error.value = ''

  try {
    if (tip.value === null) tip.value = await fetchTip()
    // Page 1 re-anchors to the freshest tip; deeper pages keep a stable window
    if (page === 1 || anchorTip.value === null) anchorTip.value = tip.value

    const end = anchorTip.value - (page - 1) * PAGE_SIZE
    if (end < 0) {
      blocks.value = []
      currentPage.value = page
      return
    }
    const start = Math.max(0, end - PAGE_SIZE + 1)
    const heights = []
    for (let h = end; h >= start; h--) heights.push(h)

    const rows = await fetchBlockRows(heights)
    if (!alive) return
    if (rows.length === 0) throw new Error('No blocks could be loaded')

    blocks.value = rows
    currentPage.value = page
  } catch (err) {
    if (!alive) return
    error.value = err?.message || 'Failed to load blocks'
  } finally {
    if (alive) loading.value = false
  }
}

// ---------------------------------------------------------------------------
// New-block awareness (poll /api/v2/status; prepend on page 1)
// ---------------------------------------------------------------------------

const pollStatus = async () => {
  if (polling) return
  polling = true
  try {
    const newTip = await fetchTip()
    if (!alive || tip.value === null || newTip <= tip.value) {
      tip.value = tip.value === null ? newTip : Math.max(tip.value, newTip)
      return
    }

    const prevTip = tip.value
    tip.value = newTip

    if (currentPage.value !== 1 || loading.value) return

    const heights = []
    for (let h = newTip; h > prevTip && heights.length < PAGE_SIZE; h--) heights.push(h)

    const fresh = await fetchBlockRows(heights)
    if (!alive || fresh.length === 0 || currentPage.value !== 1) return

    const known = new Set(blocks.value.map(b => b.height))
    const toPrepend = fresh.filter(b => !known.has(b.height))
    if (toPrepend.length === 0) return

    anchorTip.value = newTip
    blocks.value = [...toPrepend, ...blocks.value].slice(0, PAGE_SIZE)
  } catch {
    // Keep the current view on poll errors; next poll will retry
  } finally {
    polling = false
  }
}

// ---------------------------------------------------------------------------
// Summary band
// ---------------------------------------------------------------------------

const pageTxTotal = computed(() =>
  blocks.value.reduce((sum, b) => sum + (b.txCount || 0), 0)
)

const avgInterval = computed(() => {
  if (blocks.value.length < 2) return '—'
  const newest = blocks.value[0].time
  const oldest = blocks.value[blocks.value.length - 1].time
  const seconds = (newest - oldest) / (blocks.value.length - 1)
  if (!Number.isFinite(seconds) || seconds < 0) return '—'
  return `${Math.round(seconds)}s`
})

const avgSizeKb = computed(() => {
  if (blocks.value.length === 0) return '—'
  const totalBytes = blocks.value.reduce((sum, b) => sum + (b.size || 0), 0)
  return `${(totalBytes / blocks.value.length / 1024).toFixed(2)} KB`
})

// ---------------------------------------------------------------------------
// Pagination + jump
// ---------------------------------------------------------------------------

const totalPages = computed(() => {
  if (anchorTip.value === null) return 1
  return Math.max(1, Math.ceil((anchorTip.value + 1) / PAGE_SIZE))
})

const pageRange = computed(() => {
  if (blocks.value.length === 0) return ''
  const first = blocks.value[0].height
  const last = blocks.value[blocks.value.length - 1].height
  return `#${formatNumber(first)} – #${formatNumber(last)}`
})

const goToPage = (page) => {
  const target = Math.min(Math.max(1, page), totalPages.value)
  if (target === currentPage.value && blocks.value.length > 0) return
  loadPage(target)
  window.scrollTo({ top: 0, behavior: 'smooth' })
}

const jumpToHeight = () => {
  const raw = String(jumpInput.value || '').trim()
  if (!/^\d+$/.test(raw)) return
  const height = parseInt(raw, 10)
  if (!Number.isFinite(height) || height < 0) return
  router.push(`/block/${height}`)
}

const retry = () => {
  loadPage(currentPage.value)
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

onMounted(() => {
  loadPage(1)
  statusTimer = setInterval(pollStatus, STATUS_POLL_MS)
  tickTimer = setInterval(() => {
    tick.value++
  }, AGE_TICK_MS)
})

onUnmounted(() => {
  alive = false
  if (statusTimer) clearInterval(statusTimer)
  if (tickTimer) clearInterval(tickTimer)
  statusTimer = null
  tickTimer = null
})
</script>

<style scoped>
.block-list-page {
  padding: var(--space-6);
}

.page-header {
  margin-bottom: var(--space-6);
}

.page-header h1 {
  margin-bottom: var(--space-2);
}

.page-subtitle {
  color: var(--text-secondary);
  font-size: var(--text-lg);
}

/* Summary band */
.summary-band {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: var(--space-4);
  padding: var(--space-3) var(--space-5);
  margin-bottom: var(--space-4);
  border-radius: var(--radius-md);
  background: var(--glass-bg);
  border: 1px solid var(--glass-border);
  backdrop-filter: blur(var(--blur-sm));
  -webkit-backdrop-filter: blur(var(--blur-sm));
  box-shadow: var(--shadow-xs), var(--glass-highlight);
}

.summary-item {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.summary-label {
  font-size: var(--text-2xs);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
  color: var(--text-tertiary);
  white-space: nowrap;
}

.summary-value {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
  font-size: var(--text-base);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
  white-space: nowrap;
}

.summary-accent {
  color: var(--text-accent);
}

/* Table card */
.table-card {
  border-radius: var(--radius-md);
  background: var(--glass-bg);
  border: 1px solid var(--glass-border);
  backdrop-filter: blur(var(--blur-sm));
  -webkit-backdrop-filter: blur(var(--blur-sm));
  box-shadow: var(--shadow-xs), var(--glass-highlight);
  overflow: hidden;
}

.table-scroll {
  overflow-x: auto;
  scrollbar-width: thin;
  scrollbar-color: var(--purple-mid) transparent;
}

.table-scroll::-webkit-scrollbar {
  height: 6px;
}

.table-scroll::-webkit-scrollbar-thumb {
  background: var(--purple-mid);
  border-radius: var(--radius-full);
}

.block-table {
  width: 100%;
  border-collapse: collapse;
}

.block-table th {
  padding: var(--space-3) var(--space-4);
  font-size: var(--text-2xs);
  font-weight: var(--weight-bold);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
  color: var(--text-tertiary);
  border-bottom: 1px solid var(--glass-border);
  white-space: nowrap;
}

.th-left {
  text-align: left;
}

.th-right {
  text-align: right;
}

/* Skeleton + empty states */
.skeleton-row {
  border-bottom: 1px solid var(--border-secondary);
}

.skeleton-row:last-child {
  border-bottom: none;
}

.skeleton-cell {
  padding: var(--space-3) var(--space-4);
}

.empty-cell {
  padding: var(--space-8);
  text-align: center;
  color: var(--text-tertiary);
  font-style: italic;
}

.error-container {
  min-height: 320px;
  display: flex;
  align-items: center;
  justify-content: center;
}

/* New block entrance animation (timeline conventions) */
.row-enter-enter-active {
  animation: row-slide-in 500ms var(--ease-out);
}

.row-enter-leave-active {
  display: none;
}

.row-enter-move {
  transition: transform 400ms var(--ease-out);
}

@keyframes row-slide-in {
  0% {
    opacity: 0;
    transform: translateY(-14px);
    background: rgba(var(--rgb-green-accent), 0.12);
  }
  60% {
    background: rgba(var(--rgb-green-accent), 0.1);
  }
  100% {
    opacity: 1;
    transform: translateY(0);
    background: transparent;
  }
}

/* Controls */
.table-controls {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
  flex-wrap: wrap;
  margin-top: var(--space-4);
}

.pager {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

.pager-info {
  font-size: var(--text-sm);
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.pager-range {
  color: var(--text-tertiary);
  font-family: var(--font-mono);
  font-size: var(--text-xs);
}

.jump-form {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.jump-input {
  width: 180px;
  padding: var(--space-2) var(--space-3);
  font-family: var(--font-mono);
  font-size: var(--text-sm);
  font-variant-numeric: tabular-nums;
  color: var(--text-primary);
  background: rgba(var(--rgb-purple-darkest), 0.5);
  border: 1px solid var(--glass-border);
  border-radius: var(--radius-md);
  outline: none;
  transition: border-color var(--transition-fast);
}

.jump-input::placeholder {
  color: var(--text-tertiary);
  font-family: var(--font-primary);
}

.jump-input:focus {
  border-color: var(--focus-ring-color);
}

@media (max-width: 768px) {
  .block-list-page {
    padding: var(--space-4);
  }

  .summary-band {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    row-gap: var(--space-3);
  }

  .table-controls {
    flex-direction: column;
    align-items: stretch;
  }

  .pager {
    justify-content: space-between;
  }

  .jump-form {
    justify-content: stretch;
  }

  .jump-input {
    flex: 1;
    width: auto;
  }
}
</style>
