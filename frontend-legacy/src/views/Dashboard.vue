<template>
  <AppLayout>
    <div class="dashboard">
      <!-- Page header: title + compact sync pill -->
      <div class="page-header">
        <div>
          <h1 class="page-title">PIVX Blockchain Explorer</h1>
          <p class="page-subtitle">Real-time network statistics and recent activity</p>
        </div>
        <div class="header-status">
          <SyncStatusPill
            :healthy="syncHealthy"
            :height="chainStore.networkHeight"
            :sync-percentage="chainStore.syncPercentage"
            :blocks-behind="chainStore.blocksBehind"
            :loading="chainStore.loading && !initialLoadComplete"
          />
          <LiveIndicator
            :connected="wsStore.anyConnected"
            :connecting="!wsStore.anyConnected && !initialLoadComplete"
            show-label
          />
        </div>
      </div>

      <!-- Prominent search -->
      <div class="hero-search">
        <SearchBar />
      </div>

      <!-- Sync progress (only while catching up) -->
      <SyncProgressCard
        v-if="initialLoadComplete && !syncHealthy"
        class="sync-progress"
        :sync-height="chainStore.syncHeight"
        :network-height="chainStore.networkHeight"
        :sync-percentage="chainStore.syncPercentage"
        :blocks-behind="chainStore.blocksBehind"
      />

      <!-- Block timeline -->
      <section class="section">
        <div class="section-header">
          <h2>Blocks</h2>
          <Button variant="ghost" size="sm" @click="$router.push('/blocks')">
            View All →
          </Button>
        </div>
        <BlockTimeline
          :blocks="timelineBlocks"
          :pending="mempoolInfo"
          :loading="timelineLoading"
          :error="timelineError"
        />
      </section>

      <!-- KPI band -->
      <section class="section">
        <div class="section-header">
          <h2>Network Overview</h2>
          <Button variant="ghost" size="sm" @click="$router.push('/analytics')">
            Analytics →
          </Button>
        </div>
        <KpiBand />
      </section>

      <!-- Latest transactions feed -->
      <section class="section">
        <div class="section-header">
          <h2>Latest Transactions</h2>
        </div>

        <div v-if="txLoading" class="loading-state">
          <SkeletonLoader variant="card" height="80px" v-for="i in 5" :key="i" />
        </div>

        <div v-else-if="txError" class="error-state">
          <p>⚠️ Failed to load recent transactions</p>
        </div>

        <div v-else-if="recentTransactions.length > 0" class="transactions-list">
          <TransitionGroup name="slide-in">
            <TransactionRow
              v-for="tx in recentTransactions"
              :key="tx.txid"
              :transaction="tx"
              :class="{ 'new-tx': isNewTransaction(tx.txid) }"
              @click="navigateToTransaction(tx)"
            />
          </TransitionGroup>
        </div>

        <div v-else class="empty-state">
          <p>No transactions available</p>
        </div>
      </section>

      <!-- Error Display -->
      <div v-if="chainStore.error" class="error-banner">
        ⚠️ {{ chainStore.error }}
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { useChainStore } from '@/stores/chainStore'
import { useWebSocketStore } from '@/stores/websocketStore'
import { detectTransactionType } from '@/utils/transactionHelpers'
import api from '@/services/api'
import { transactionService } from '@/services/transactionService'
import AppLayout from '@/components/layout/AppLayout.vue'
import SearchBar from '@/components/layout/SearchBar.vue'
import SyncStatusPill from '@/components/dashboard/SyncStatusPill.vue'
import SyncProgressCard from '@/components/dashboard/SyncProgressCard.vue'
import BlockTimeline from '@/components/dashboard/BlockTimeline.vue'
import KpiBand from '@/components/dashboard/KpiBand.vue'
import TransactionRow from '@/components/common/TransactionRow.vue'
import Button from '@/components/common/Button.vue'
import SkeletonLoader from '@/components/common/SkeletonLoader.vue'
import LiveIndicator from '@/components/common/LiveIndicator.vue'

const TIMELINE_SIZE = 8
const POLL_INTERVAL_MS = 10000
const FEED_SIZE = 8

const router = useRouter()
const chainStore = useChainStore()
const wsStore = useWebSocketStore()

const initialLoadComplete = ref(false)

// Considered "in sync" unless explicitly not synced or trailing by more than 5 blocks
const syncHealthy = computed(() => {
  return chainStore.synced && chainStore.blocksBehind <= 5
})

// ---------------------------------------------------------------------------
// Block timeline state (latest blocks + pending mempool tile)
// ---------------------------------------------------------------------------
const timelineBlocks = ref([])
const timelineLoading = ref(true)
const timelineError = ref(false)
const mempoolInfo = ref({ txCount: 0, bytes: 0 })
let timelineRefreshing = false
let queuedTipHeight = null

/** Map a /api/v2/block-detail response to the tile model. */
const mapBlockDetail = (data) => {
  const txs = Array.isArray(data.tx) ? data.tx : []
  const coinstake = txs.find((t) => t && t.tx_type === 'coinstake')
  const stakerVin = coinstake?.vin?.[0]
  return {
    height: data.height,
    hash: data.hash,
    time: data.time,
    txCount: txs.length,
    size: data.size || 0,
    staker: stakerVin?.address || stakerVin?.addresses?.[0] || null,
    txids: txs.map((t) => t?.txid).filter(Boolean)
  }
}

/** Fetch the latest TIMELINE_SIZE blocks in parallel, reusing already-loaded ones. */
const refreshTimeline = async (tipHeight) => {
  if (!tipHeight || tipHeight <= 0) return
  if (timelineRefreshing) {
    // Remember the newest requested tip; re-run once the current refresh ends
    queuedTipHeight = Math.max(queuedTipHeight || 0, tipHeight)
    return
  }
  timelineRefreshing = true
  try {
    const heights = []
    for (let i = 0; i < TIMELINE_SIZE; i++) {
      const h = tipHeight - i
      if (h >= 0) heights.push(h)
    }

    const cache = new Map(timelineBlocks.value.map((b) => [b.height, b]))
    const missing = heights.filter((h) => !cache.has(h))

    const results = await Promise.allSettled(
      missing.map((h) => api.get(`/api/v2/block-detail/${h}`))
    )
    results.forEach((res) => {
      if (res.status === 'fulfilled' && res.value?.data?.height !== undefined) {
        const block = mapBlockDetail(res.value.data)
        cache.set(block.height, block)
      }
    })

    const blocks = heights.map((h) => cache.get(h)).filter(Boolean)
    timelineBlocks.value = blocks
    timelineError.value = blocks.length === 0
  } catch {
    timelineError.value = timelineBlocks.value.length === 0
  } finally {
    timelineRefreshing = false
    timelineLoading.value = false
    if (queuedTipHeight && queuedTipHeight > tipHeight) {
      const next = queuedTipHeight
      queuedTipHeight = null
      refreshTimeline(next)
    } else {
      queuedTipHeight = null
    }
  }
}

const fetchMempool = async () => {
  try {
    const response = await api.get('/api/v2/mempool')
    const data = response.data || {}
    mempoolInfo.value = {
      txCount: data.size ?? (Array.isArray(data.transactions) ? data.transactions.length : 0),
      bytes: data.bytes || 0
    }
  } catch {
    // Keep last known mempool snapshot on error
  }
}

// ---------------------------------------------------------------------------
// Latest transactions feed
// ---------------------------------------------------------------------------
const recentTransactions = ref([])
const txLoading = ref(true)
const txError = ref(false)
const newTxIds = ref(new Set())
const txAnimationTimers = new Map()

const isNewTransaction = (txid) => newTxIds.value.has(txid)

const markTransactionNew = (txid) => {
  if (txAnimationTimers.has(txid)) clearTimeout(txAnimationTimers.get(txid))
  newTxIds.value.add(txid)
  const timer = setTimeout(() => {
    newTxIds.value.delete(txid)
    txAnimationTimers.delete(txid)
  }, 2000)
  txAnimationTimers.set(txid, timer)
}

const isValidFeedTx = (tx) => {
  const height = tx.blockHeight || tx.height
  return height !== -1 && height !== -2
}

/** Seed the feed from txids already fetched for the timeline (no extra block calls). */
const fetchRecentTransactions = async () => {
  txLoading.value = true
  txError.value = false
  try {
    const txids = timelineBlocks.value.flatMap((b) => b.txids || []).slice(0, FEED_SIZE)
    const transactions = await transactionService.getTransactions(txids)
    recentTransactions.value = transactions
      .filter(isValidFeedTx)
      .map((tx) => ({ ...tx, type: detectTransactionType(tx) }))
    txError.value = txids.length > 0 && recentTransactions.value.length === 0
  } catch {
    txError.value = true
  } finally {
    txLoading.value = false
  }
}

const handleNewTransaction = async (txEvent) => {
  try {
    const fullTx = await transactionService.getTransaction(txEvent.txid)
    if (!isValidFeedTx(fullTx)) return
    markTransactionNew(txEvent.txid)
    recentTransactions.value = [
      { ...fullTx, type: detectTransactionType(fullTx) },
      ...recentTransactions.value.filter((t) => t.txid !== txEvent.txid)
    ].slice(0, FEED_SIZE)
  } catch {
    // Skip transactions that fail to resolve
  }
}

// ---------------------------------------------------------------------------
// Polling + WebSocket wiring
// ---------------------------------------------------------------------------
let pollTimer = null
let unsubscribeBlock = null
let unsubscribeTx = null

// Any networkHeight change (poll or WebSocket-triggered) refreshes the strip
watch(
  () => chainStore.networkHeight,
  (height, oldHeight) => {
    if (height > 0 && height !== oldHeight && initialLoadComplete.value) {
      refreshTimeline(height)
    }
  }
)

// Reorg: drop cached blocks and rebuild everything
watch(
  () => chainStore.reorgDetected,
  (detected) => {
    if (!detected) return
    timelineBlocks.value = []
    timelineLoading.value = true
    newTxIds.value.clear()
    refreshTimeline(chainStore.networkHeight).then(fetchRecentTransactions)
  }
)

onMounted(async () => {
  await chainStore.fetchChainState()
  await Promise.all([refreshTimeline(chainStore.networkHeight), fetchMempool()])
  initialLoadComplete.value = true

  // Feed seeds from the timeline's tx data
  fetchRecentTransactions()

  // Poll status + mempool every 10s; the height watcher refreshes the strip
  pollTimer = setInterval(() => {
    chainStore.fetchChainState()
    fetchMempool()
  }, POLL_INTERVAL_MS)

  // WebSocket push for instant updates between polls
  wsStore.connectBlocks()
  wsStore.connectTransactions()
  unsubscribeBlock = wsStore.onNewBlock(() => {
    chainStore.fetchChainState()
    fetchMempool()
  })
  unsubscribeTx = wsStore.onNewTransaction(handleNewTransaction)
})

onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer)
  if (unsubscribeBlock) unsubscribeBlock()
  if (unsubscribeTx) unsubscribeTx()

  txAnimationTimers.forEach(clearTimeout)
  txAnimationTimers.clear()
  newTxIds.value.clear()
})

const navigateToTransaction = (tx) => {
  router.push(`/tx/${tx.txid}`)
}
</script>

<style scoped>
.dashboard {
  padding: var(--space-6) 0;
  display: grid;
  gap: var(--space-8);
}

.page-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--space-4);
}

.page-title {
  margin-bottom: var(--space-2);
  text-align: left;
}

.page-subtitle {
  text-align: left;
  color: var(--text-secondary);
}

.header-status {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  flex-shrink: 0;
}

.hero-search {
  max-width: 640px;
}

.sync-progress {
  margin-top: calc(-1 * var(--space-4));
}

.section {
  display: grid;
  gap: var(--space-4);
}

.section-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.section-header h2 {
  margin: 0;
  color: var(--text-primary);
}

.transactions-list {
  display: grid;
  gap: var(--space-3);
}

.loading-state,
.error-state,
.empty-state {
  display: grid;
  gap: var(--space-3);
  padding: var(--space-6);
}

.error-state,
.empty-state {
  text-align: center;
  color: var(--text-tertiary);
  font-style: italic;
}

.error-banner {
  background: rgba(239, 68, 68, 0.1);
  border: 1px solid var(--danger);
  border-radius: var(--radius-md);
  padding: var(--space-4);
  color: var(--text-primary);
}

/* Slide-in animation for new transactions */
.slide-in-enter-active {
  animation: slideInFromTop 0.5s ease-out;
}

.slide-in-leave-active {
  display: none;
}

@keyframes slideInFromTop {
  from {
    opacity: 0;
    transform: translateY(-20px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

/* Highlight new items with a glow */
.new-tx {
  animation: glow 2s ease-in-out;
}

@keyframes glow {
  0%, 100% {
    box-shadow: 0 0 0 rgba(179, 255, 120, 0);
  }
  50% {
    box-shadow: 0 0 20px rgba(179, 255, 120, 0.5);
  }
}

/* Mobile responsiveness */
@media (max-width: 768px) {
  .dashboard {
    gap: var(--space-6);
  }

  .page-header {
    flex-direction: column;
    align-items: flex-start;
  }

  .header-status {
    flex-direction: row;
    flex-wrap: wrap;
  }

  .hero-search {
    max-width: 100%;
  }
}
</style>
