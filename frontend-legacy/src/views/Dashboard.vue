<template>
  <AppLayout>
    <div class="dashboard">
      <div class="page-header">
        <div>
          <h1 class="page-title">PIVX Blockchain Explorer</h1>
          <p class="page-subtitle">Real-time network statistics and recent activity</p>
        </div>
        <LiveIndicator 
          :connected="wsStore.anyConnected" 
          :connecting="!wsStore.anyConnected && !initialLoadComplete"
          show-label 
        />
      </div>

      <!-- Network Stats -->
      <div class="stats-grid">
        <StatCard
          label="Network Height"
          :value="formatNumber(chainStore.networkHeight)"
          icon="📦"
          :isLoading="chainStore.loading"
        />
        <StatCard
          label="Indexed Height"
          :value="formatNumber(chainStore.syncHeight)"
          icon="💾"
          :isLoading="chainStore.loading"
        />
        <StatCard
          label="Sync Progress"
          :value="formatPercentage(chainStore.syncPercentage) + '%'"
          :subtitle="chainStore.synced ? 'Fully Synced' : 'Syncing...'"
          :valueClass="chainStore.synced ? 'text-success' : 'text-warning'"
          icon="🔄"
          :isLoading="chainStore.loading"
        />
        <StatCard
          label="Blocks Behind"
          :value="formatNumber(chainStore.blocksBehind)"
          :subtitle="chainStore.blocksBehind === 0 ? 'Up to date' : 'Catching up'"
          :valueClass="chainStore.blocksBehind === 0 ? 'text-success' : 'text-warning'"
          icon="⏱️"
          :isLoading="chainStore.loading"
        />
      </div>

      <!-- Recent Blocks Section -->
      <div class="content-sections">
        <section class="section">
          <div class="section-header">
            <h2>Recent Blocks</h2>
            <Button variant="ghost" size="sm" @click="$router.push('/blocks')">
              View All →
            </Button>
          </div>
          
          <div v-if="blocksLoading" class="loading-state">
            <SkeletonLoader variant="card" v-for="i in 5" :key="i" />
          </div>
          
          <div v-else-if="blocksError" class="error-state">
            <p>⚠️ Failed to load recent blocks</p>
          </div>
          
          <div v-else-if="recentBlocks.length > 0" class="blocks-grid">
            <TransitionGroup name="slide-in">
              <BlockCard
                v-for="block in recentBlocks"
                :key="block.height"
                :block="block"
                :class="{ 'new-block': isNewBlock(block.height) }"
                @click="navigateToBlock(block)"
              />
            </TransitionGroup>
          </div>
          
          <div v-else class="empty-state">
            <p>No blocks available</p>
          </div>
        </section>

        <!-- Recent Transactions Section -->
        <section class="section">
          <div class="section-header">
            <h2>Recent Transactions</h2>
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
      </div>

      <!-- Error Display -->
      <div v-if="chainStore.error" class="error-banner">
        ⚠️ {{ chainStore.error }}
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, onMounted, onUnmounted, watch } from 'vue'
import { useRouter } from 'vue-router'
import { useChainStore } from '@/stores/chainStore'
import { useWebSocketStore } from '@/stores/websocketStore'
import { formatNumber, formatPercentage } from '@/utils/formatters'
import { detectTransactionType } from '@/utils/transactionHelpers'
import { blockService } from '@/services/blockService'
import { transactionService } from '@/services/transactionService'
import AppLayout from '@/components/layout/AppLayout.vue'
import StatCard from '@/components/common/StatCard.vue'
import BlockCard from '@/components/common/BlockCard.vue'
import TransactionRow from '@/components/common/TransactionRow.vue'
import Button from '@/components/common/Button.vue'
import SkeletonLoader from '@/components/common/SkeletonLoader.vue'
import LiveIndicator from '@/components/common/LiveIndicator.vue'

const router = useRouter()
const chainStore = useChainStore()
const wsStore = useWebSocketStore()

// Recent blocks state
const recentBlocks = ref([])
const blocksLoading = ref(false)
const blocksError = ref(false)

// Recent transactions state
const recentTransactions = ref([])
const txLoading = ref(false)
const txError = ref(false)

// WebSocket state
const initialLoadComplete = ref(false)
const newBlockHeights = ref(new Set())
const newTxIds = ref(new Set())
const blockAnimationTimers = ref(new Map()) // Track timers for cleanup
const txAnimationTimers = ref(new Map()) // Track timers for cleanup
let unsubscribeBlock = null
let unsubscribeTx = null

// Track if block/tx is new (for animation)
const isNewBlock = (height) => newBlockHeights.value.has(height)
const isNewTransaction = (txid) => newTxIds.value.has(txid)

// Fetch recent blocks
const fetchRecentBlocks = async () => {
  blocksLoading.value = true
  blocksError.value = false
  try {
    const result = await blockService.getRecentBlocks(5)
    
    // Check if any blocks failed to load
    if (result.errors && result.errors.length > 0) {
      console.warn(`⚠️ ${result.errors.length} blocks failed to load:`, result.errors)
    }
    
    // Transform the blocks to match our component's expected format
    recentBlocks.value = result.map(block => ({
      ...block,
      txCount: block.tx?.length || 0,
      // Size is not in the basic API response, skip it
    }))
    
    // Only set error state if ALL blocks failed
    if (recentBlocks.value.length === 0) {
      blocksError.value = true
    }
  } catch (error) {
    console.error('Failed to fetch recent blocks:', error)
    blocksError.value = true
  } finally {
    blocksLoading.value = false
  }
}

// Fetch recent transactions from the latest blocks
const fetchRecentTransactions = async () => {
  txLoading.value = true
  txError.value = false
  try {
    // Get the 3 most recent blocks
    const blocks = await blockService.getRecentBlocks(3)
    
    // Extract all transaction IDs from these blocks
    const allTxIds = blocks.flatMap(block => block.tx || [])
    
    // Fetch transaction details for the first 10 txids
    const txidsToFetch = allTxIds.slice(0, 10)
    const transactions = await transactionService.getTransactions(txidsToFetch)
    
    // Detect transaction types and filter out invalid heights (-1 orphaned, -2 unresolved)
    recentTransactions.value = transactions
      .filter(tx => {
        const height = tx.blockHeight || tx.height
        return height !== -1 && height !== -2
      })
      .map(tx => ({
        ...tx,
        type: detectTransactionType(tx)
      }))
  } catch (error) {
    console.error('Failed to fetch recent transactions:', error)
    txError.value = true
  } finally {
    txLoading.value = false
  }
}

// Handle new block from WebSocket
const handleNewBlock = async (blockEvent) => {
  console.log('New block received:', blockEvent)
  
  // Cancel existing timer for this height (prevents duplicates)
  if (blockAnimationTimers.value.has(blockEvent.height)) {
    clearTimeout(blockAnimationTimers.value.get(blockEvent.height))
  }
  
  // Enforce size limit to prevent unbounded growth
  if (newBlockHeights.value.size > 100) {
    console.warn('⚠️ Block animation set exceeded 100 entries, clearing old entries')
    // Clear oldest entries (keep newest 50)
    const entries = Array.from(newBlockHeights.value)
    entries.slice(0, entries.length - 50).forEach(h => {
      newBlockHeights.value.delete(h)
      if (blockAnimationTimers.value.has(h)) {
        clearTimeout(blockAnimationTimers.value.get(h))
        blockAnimationTimers.value.delete(h)
      }
    })
  }
  
  // Mark as new for animation
  newBlockHeights.value.add(blockEvent.height)
  
  const timer = setTimeout(() => {
    newBlockHeights.value.delete(blockEvent.height)
    blockAnimationTimers.value.delete(blockEvent.height)
  }, 2000)
  
  blockAnimationTimers.value.set(blockEvent.height, timer)
  
  // Fetch the full block details
  try {
    const fullBlock = await blockService.getBlock(blockEvent.height)
    const formattedBlock = {
      ...fullBlock,
      txCount: fullBlock.tx?.length || 0
    }
    
    // Add to the beginning of the list and keep only 5
    recentBlocks.value = [formattedBlock, ...recentBlocks.value].slice(0, 5)
    
    // Update chain height
    await chainStore.fetchChainState()
  } catch (error) {
    console.error('Failed to fetch new block details:', error)
    // Immediate cleanup on error
    newBlockHeights.value.delete(blockEvent.height)
    if (blockAnimationTimers.value.has(blockEvent.height)) {
      clearTimeout(blockAnimationTimers.value.get(blockEvent.height))
      blockAnimationTimers.value.delete(blockEvent.height)
    }
  }
}

// Handle new transaction from WebSocket
const handleNewTransaction = async (txEvent) => {
  console.log('New transaction received:', txEvent)
  
  // Cancel existing timer for this txid (prevents duplicates)
  if (txAnimationTimers.value.has(txEvent.txid)) {
    clearTimeout(txAnimationTimers.value.get(txEvent.txid))
  }
  
  // Enforce size limit to prevent unbounded growth
  if (newTxIds.value.size > 100) {
    console.warn('⚠️ Transaction animation set exceeded 100 entries, clearing old entries')
    // Clear oldest entries (keep newest 50)
    const entries = Array.from(newTxIds.value)
    entries.slice(0, entries.length - 50).forEach(txid => {
      newTxIds.value.delete(txid)
      if (txAnimationTimers.value.has(txid)) {
        clearTimeout(txAnimationTimers.value.get(txid))
        txAnimationTimers.value.delete(txid)
      }
    })
  }
  
  // Mark as new for animation
  newTxIds.value.add(txEvent.txid)
  
  const timer = setTimeout(() => {
    newTxIds.value.delete(txEvent.txid)
    txAnimationTimers.value.delete(txEvent.txid)
  }, 2000)
  
  txAnimationTimers.value.set(txEvent.txid, timer)
  
  // Fetch the full transaction details
  try {
    const fullTx = await transactionService.getTransaction(txEvent.txid)
    
    // Filter out invalid heights (-1 orphaned, -2 unresolved)
    const height = fullTx.blockHeight || fullTx.height
    if (height === -1 || height === -2) {
      // Cleanup animation for invalid transactions
      newTxIds.value.delete(txEvent.txid)
      if (txAnimationTimers.value.has(txEvent.txid)) {
        clearTimeout(txAnimationTimers.value.get(txEvent.txid))
        txAnimationTimers.value.delete(txEvent.txid)
      }
      return // Don't add orphaned or unresolved transactions
    }
    
    const formattedTx = {
      ...fullTx,
      type: detectTransactionType(fullTx)
    }
    
    // Add to the beginning of the list and keep only 10
    recentTransactions.value = [formattedTx, ...recentTransactions.value].slice(0, 10)
  } catch (error) {
    console.error('Failed to fetch new transaction details:', error)
    // Immediate cleanup on error
    newTxIds.value.delete(txEvent.txid)
    if (txAnimationTimers.value.has(txEvent.txid)) {
      clearTimeout(txAnimationTimers.value.get(txEvent.txid))
      txAnimationTimers.value.delete(txEvent.txid)
    }
  }
}

onMounted(async () => {
  // Fetch initial data
  await chainStore.fetchChainState()
  await Promise.all([
    fetchRecentBlocks(),
    fetchRecentTransactions()
  ])
  
  initialLoadComplete.value = true
  
  // Connect to WebSocket and subscribe to events
  wsStore.connectBlocks()
  wsStore.connectTransactions()
  
  unsubscribeBlock = wsStore.onNewBlock(handleNewBlock)
  unsubscribeTx = wsStore.onNewTransaction(handleNewTransaction)
})

// Watch for reorg detection and refetch data
watch(() => chainStore.reorgDetected, (detected) => {
  if (detected) {
    console.log('🔄 Reorg detected - clearing cache and refetching data')
    // Clear cached data
    recentBlocks.value = []
    recentTransactions.value = []
    newBlockHeights.value.clear()
    newTxIds.value.clear()
    
    // Refetch all data
    Promise.all([
      fetchRecentBlocks(),
      fetchRecentTransactions()
    ])
  }
})

onUnmounted(() => {
  // Clean up WebSocket subscriptions
  if (unsubscribeBlock) unsubscribeBlock()
  if (unsubscribeTx) unsubscribeTx()
  
  // Clear all animation timers to prevent memory leaks
  blockAnimationTimers.value.forEach(clearTimeout)
  blockAnimationTimers.value.clear()
  txAnimationTimers.value.forEach(clearTimeout)
  txAnimationTimers.value.clear()
  
  // Clear animation sets
  newBlockHeights.value.clear()
  newTxIds.value.clear()
})

// Navigation handlers
const navigateToBlock = (block) => {
  router.push(`/block/${block.height}`)
}

const navigateToTransaction = (tx) => {
  router.push(`/tx/${tx.txid}`)
}
</script>

<style scoped>
.dashboard {
  padding: var(--space-6) 0;
}

.page-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--space-8);
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

.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
  gap: var(--space-4);
  margin-bottom: var(--space-8);
}

.content-sections {
  display: grid;
  gap: var(--space-8);
  margin-top: var(--space-8);
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

.blocks-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  gap: var(--space-4);
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
  margin-top: var(--space-4);
  color: var(--text-primary);
}

/* Slide-in animation for new blocks/transactions */
.slide-in-enter-active {
  animation: slideInFromTop 0.5s ease-out;
}

.slide-in-leave-active {
  animation: slideOutToBottom 0.3s ease-in;
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

@keyframes slideOutToBottom {
  from {
    opacity: 1;
    transform: translateY(0);
  }
  to {
    opacity: 0;
    transform: translateY(20px);
  }
}

/* Highlight new items with a glow */
.new-block,
.new-tx {
  animation: glow 2s ease-in-out;
}

@keyframes glow {
  0%, 100% {
    box-shadow: 0 0 0 rgba(89, 252, 179, 0);
  }
  50% {
    box-shadow: 0 0 20px rgba(89, 252, 179, 0.5);
  }
}

/* Mobile responsiveness */
@media (max-width: 768px) {
  .page-header {
    flex-direction: column;
    align-items: flex-start;
  }
  
  .page-title {
    text-align: left;
  }
  
  .page-subtitle {
    text-align: left;
  }

  .stats-grid {
    grid-template-columns: 1fr;
  }
}
</style>
