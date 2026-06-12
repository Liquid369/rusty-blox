<template>
  <AppLayout>
    <div class="block-detail-page">
      <!-- Loading State -->
      <div v-if="loading" class="loading-container">
        <LoadingSpinner size="lg" text="Loading block..." />
      </div>

      <!-- Error State -->
      <div v-else-if="error" class="error-container">
        <EmptyState
          icon="⚠️"
          title="Block Not Found"
          :message="error"
        >
          <template #action>
            <Button @click="$router.push('/blocks')">View All Blocks</Button>
          </template>
        </EmptyState>
      </div>

      <!-- Block Details -->
      <div v-else-if="block">
        <!-- Header Navigation -->
        <div class="block-navigation">
          <Button
            v-if="block.height > 0"
            variant="ghost"
            @click="navigateToBlock(block.height - 1)"
          >
            ← Previous Block
          </Button>
          <h1 class="block-title">Block #{{ formatNumber(block.height) }}</h1>
          <Button
            v-if="block.height < chainStore.syncHeight"
            variant="ghost"
            @click="navigateToBlock(block.height + 1)"
          >
            Next Block →
          </Button>
        </div>

        <!-- Block Info Card -->
        <Card class="block-info-card">
          <template #header>
            <div class="card-header-content">
              <span>Block Information</span>
              <Badge :variant="block.isPoS ? 'accent' : 'primary'">
                {{ block.isPoS ? 'Proof of Stake' : 'Proof of Work' }}
              </Badge>
            </div>
          </template>

          <div class="info-grid">
            <InfoRow label="Block Hash" icon="🔗">
              <HashDisplay :hash="block.hash" show-copy />
            </InfoRow>

            <InfoRow label="Height" icon="📊">
              {{ formatNumber(block.height) }}
            </InfoRow>

            <InfoRow label="Timestamp" icon="🕐">
              <div class="timestamp-group">
                <span>{{ formatDate(block.time) }}</span>
                <span class="time-ago">{{ formatTimeAgo(block.time) }}</span>
              </div>
            </InfoRow>

            <InfoRow label="Confirmations" icon="✅">
              <div class="confirmation-display">
                <Badge :variant="confirmations >= 6 ? 'success' : 'warning'">
                  {{ formatNumber(confirmations) }}
                </Badge>
                <Badge v-if="isSyncLagging" variant="warning" class="sync-warning">
                  ⚠️ Node syncing
                </Badge>
              </div>
            </InfoRow>

            <InfoRow label="Transactions" icon="📝">
              {{ block.txCount }} transaction{{ block.txCount !== 1 ? 's' : '' }}
            </InfoRow>

            <InfoRow v-if="blockInterval !== null" label="Block Interval" icon="⏱️">
              <div class="interval-display">
                <span class="interval-value">{{ formatDuration(blockInterval) }}</span>
                <span class="interval-note">since block #{{ formatNumber(block.height - 1) }}</span>
              </div>
            </InfoRow>

            <InfoRow v-if="blockReward !== null" label="Block Reward" icon="🏆">
              <span class="reward-value">{{ formatPIV(blockReward) }} PIV</span>
            </InfoRow>

            <InfoRow v-if="totalTxSize > 0" label="Size (approx)" icon="💾">
              <span class="size-value">{{ formatBytes(totalTxSize + 80) }}</span>
            </InfoRow>

            <InfoRow label="Difficulty" icon="⚡">
              {{ formatDifficulty(block.difficulty) }}
            </InfoRow>

            <InfoRow label="Version" icon="🔢">
              {{ block.version }}
            </InfoRow>

            <InfoRow label="Merkle Root" icon="🌳">
              <HashDisplay :hash="block.merkleroot" :truncate="true" show-copy />
            </InfoRow>

            <InfoRow label="Nonce" icon="🎲">
              {{ formatNumber(block.nonce) }}
            </InfoRow>

            <InfoRow label="Bits" icon="🔧">
              {{ block.bits }}
            </InfoRow>

            <InfoRow v-if="block.previousblockhash" label="Previous Block" icon="⬅️">
              <HashDisplay
                :hash="block.previousblockhash"
                :truncate="true"
                show-copy
                :link-to="`/block/${block.height - 1}`"
              />
            </InfoRow>

            <InfoRow v-if="block.nextblockhash" label="Next Block" icon="➡️">
              <HashDisplay
                :hash="block.nextblockhash"
                :truncate="true"
                show-copy
                :link-to="`/block/${block.height + 1}`"
              />
            </InfoRow>
          </div>
        </Card>

        <!-- Transactions Section -->
        <div class="transactions-section">
          <h2 class="section-title">
            Transactions ({{ block.txCount }})
          </h2>

          <!-- Composition Summary -->
          <Card v-if="!loadingTransactions && transactions.length > 0" class="composition-card">
            <div class="composition-band">
              <div class="composition-types">
                <div
                  v-for="entry in txComposition"
                  :key="entry.type"
                  class="composition-chip"
                >
                  <Badge :variant="entry.variant" size="sm">{{ entry.label }}</Badge>
                  <span class="composition-count">{{ entry.count }}</span>
                </div>
              </div>
              <div class="composition-total">
                <span class="composition-total-label">Total Value Transferred</span>
                <span class="composition-total-value">{{ formatPIV(totalValueTransferred) }} PIV</span>
              </div>
            </div>
          </Card>

          <!-- Loading Transactions -->
          <div v-if="loadingTransactions" class="transactions-loading">
            <SkeletonLoader variant="card" v-for="i in 3" :key="i" />
          </div>

          <!-- Transaction List -->
          <div v-else-if="transactions.length > 0" class="transactions-list">
            <TransactionRow
              v-for="tx in transactions"
              :key="tx.txid"
              :transaction="tx"
              @click="navigateToTransaction(tx)"
            />
          </div>

          <!-- No Transactions -->
          <EmptyState
            v-else
            icon="📭"
            title="No Transactions"
            message="This block contains no transactions."
          />
        </div>
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, computed, onMounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useChainStore } from '@/stores/chainStore'
import { blockService } from '@/services/blockService'
import { transactionService } from '@/services/transactionService'
import { formatNumber, formatDate, formatTimeAgo, formatBytes, formatDifficulty, formatDuration, formatPIV } from '@/utils/formatters'
import {
  detectTransactionType,
  getTransactionTypeLabel,
  getTransactionTypeBadgeVariant,
  toSats
} from '@/utils/transactionHelpers'
import { LAST_POW_BLOCK, TX_TYPES } from '@/utils/constants'
import AppLayout from '@/components/layout/AppLayout.vue'
import Card from '@/components/common/Card.vue'
import Badge from '@/components/common/Badge.vue'
import Button from '@/components/common/Button.vue'
import InfoRow from '@/components/common/InfoRow.vue'
import HashDisplay from '@/components/common/HashDisplay.vue'
import LoadingSpinner from '@/components/common/LoadingSpinner.vue'
import SkeletonLoader from '@/components/common/SkeletonLoader.vue'
import EmptyState from '@/components/common/EmptyState.vue'
import TransactionRow from '@/components/common/TransactionRow.vue'

const route = useRoute()
const router = useRouter()
const chainStore = useChainStore()

const block = ref(null)
const transactions = ref([])
const loading = ref(false)
const loadingTransactions = ref(false)
const error = ref('')
const previousBlockTime = ref(null)

// Seconds elapsed since the previous block (fetched from /api/v2/block/{h-1})
const blockInterval = computed(() => {
  if (!block.value?.time || !previousBlockTime.value) return null
  const delta = block.value.time - previousBlockTime.value
  return delta >= 0 ? delta : null
})

// Transaction type composition (n coinstake / coinbase / transparent / shield / ...)
const txComposition = computed(() => {
  const counts = new Map()
  for (const tx of transactions.value) {
    const type = tx.type || TX_TYPES.REGULAR
    counts.set(type, (counts.get(type) || 0) + 1)
  }
  const order = [
    TX_TYPES.COINBASE,
    TX_TYPES.COINSTAKE,
    TX_TYPES.BUDGET,
    TX_TYPES.SAPLING,
    TX_TYPES.COLDSTAKE,
    TX_TYPES.REGULAR
  ]
  return order
    .filter(type => counts.has(type))
    .map(type => ({
      type,
      count: counts.get(type),
      label: getTransactionTypeLabel(type),
      variant: getTransactionTypeBadgeVariant(type)
    }))
})

// Sum of all transaction output values in the block (satoshis)
const totalValueTransferred = computed(() => {
  return transactions.value
    .reduce((sum, tx) => sum + toSats(tx.value), 0)
    .toString()
})

// Sum of transaction sizes (block size approximation basis)
const totalTxSize = computed(() => {
  return transactions.value.reduce((sum, tx) => sum + (tx.size || 0), 0)
})

// Block reward, read from the coinstake (PoS) or coinbase (PoW) transaction:
// newly created value = total outputs minus total inputs
const blockReward = computed(() => {
  const rewardTx = transactions.value.find(
    tx => tx.type === TX_TYPES.COINSTAKE || tx.type === TX_TYPES.COINBASE || tx.type === TX_TYPES.BUDGET
  )
  if (!rewardTx) return null
  const minted = toSats(rewardTx.value) - toSats(rewardTx.valueIn)
  return minted > 0 ? minted.toString() : null
})

const confirmations = computed(() => {
  // Use networkHeight for actual chain depth, not syncHeight (local index)
  if (!block.value || !chainStore.networkHeight) return 0
  return Math.max(0, chainStore.networkHeight - block.value.height + 1)
})

// Detect when local index lags behind network
const syncLag = computed(() => {
  if (!chainStore.networkHeight || !chainStore.syncHeight) return 0
  return Math.max(0, chainStore.networkHeight - chainStore.syncHeight)
})

const isSyncLagging = computed(() => syncLag.value > 10)

const fetchBlock = async (identifier) => {
  loading.value = true
  error.value = ''
  block.value = null
  transactions.value = []
  previousBlockTime.value = null

  try {
    const blockData = await blockService.getBlockDetail(identifier)
    
    blockData.txCount = blockData.tx?.length || 0
    
    // Detect PoS vs PoW based on block height (from chainparams.cpp)
    // PIVX switched to PoS after block 259200
    blockData.isPoS = blockData.height > LAST_POW_BLOCK
    
    block.value = blockData

    // Fetch transactions and the previous block header (for the time delta) in parallel
    const tasks = []
    if (blockData.tx && blockData.tx.length > 0) {
      tasks.push(fetchTransactions(blockData.tx))
    }
    if (blockData.height > 0) {
      tasks.push(fetchPreviousBlockTime(blockData.height - 1))
    }
    await Promise.all(tasks)
  } catch (err) {
    error.value = err.message || 'Failed to load block'
  } finally {
    loading.value = false
  }
}

const fetchPreviousBlockTime = async (height) => {
  try {
    const prevBlock = await blockService.getBlockDetail(height)
    previousBlockTime.value = prevBlock?.time || null
  } catch {
    previousBlockTime.value = null
  }
}

const fetchTransactions = async (txids) => {
  loadingTransactions.value = true

  try {
    const txPromises = txids.map(txid => transactionService.getTransaction(txid))
    const results = await Promise.allSettled(txPromises)

    transactions.value = results
      .filter(result => result.status === 'fulfilled')
      .map(result => ({
        ...result.value,
        type: detectTransactionType(result.value)
      }))
  } finally {
    loadingTransactions.value = false
  }
}

const navigateToBlock = (height) => {
  router.push(`/block/${height}`)
}

const navigateToTransaction = (tx) => {
  router.push(`/tx/${tx.txid}`)
}

watch(() => route.params.id, (newId) => {
  if (newId) {
    fetchBlock(newId)
  }
}, { immediate: true })

// Watch for reorg detection and refetch block
watch(() => chainStore.reorgDetected, (detected) => {
  if (detected && route.params.id) {
    fetchBlock(route.params.id)
  }
})

onMounted(() => {
  chainStore.fetchChainState()
})
</script>

<style scoped>
.block-detail-page {
  padding: var(--space-6);
  max-width: 1400px;
  margin: 0 auto;
}

.block-navigation {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: var(--space-6);
  gap: var(--space-4);
}

.block-title {
  flex: 1;
  text-align: center;
  margin: 0;
}

.card-header-content {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
}

.block-info-card {
  margin-bottom: var(--space-8);
}

.info-grid {
  display: grid;
  gap: var(--space-4);
}

.timestamp-group {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.time-ago {
  font-size: var(--text-sm);
  color: var(--text-tertiary);
}

.confirmation-display {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-wrap: wrap;
}

.sync-warning {
  font-size: var(--text-xs);
}

.transactions-section {
  margin-top: var(--space-8);
}

.composition-card {
  margin-bottom: var(--space-4);
}

.composition-band {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
  flex-wrap: wrap;
}

.composition-types {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex-wrap: wrap;
}

.composition-chip {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  background: rgba(var(--rgb-purple-darkest), 0.45);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-full);
}

.composition-count {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
  font-weight: var(--weight-bold);
  font-size: var(--text-sm);
  color: var(--text-primary);
}

.composition-total {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  text-align: right;
}

.composition-total-label {
  font-size: var(--text-xs);
  font-weight: var(--weight-semibold);
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
}

.composition-total-value {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
  font-weight: var(--weight-bold);
  font-size: var(--text-lg);
  color: var(--pivx-accent);
}

.interval-display {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  align-items: flex-end;
}

.interval-value {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
  font-weight: var(--weight-bold);
}

.interval-note {
  font-size: var(--text-xs);
  color: var(--text-tertiary);
}

.reward-value,
.size-value {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
  font-weight: var(--weight-bold);
  color: var(--text-accent);
}

.section-title {
  margin-bottom: var(--space-4);
  color: var(--text-primary);
}

.transactions-loading,
.transactions-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.loading-container,
.error-container {
  min-height: 400px;
  display: flex;
  align-items: center;
  justify-content: center;
}

@media (max-width: 768px) {
  .block-detail-page {
    padding: var(--space-4);
  }

  .block-navigation {
    flex-direction: column;
    text-align: center;
  }

  .block-title {
    font-size: var(--text-2xl);
  }
}
</style>
