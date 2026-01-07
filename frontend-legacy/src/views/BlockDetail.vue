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
import { formatNumber, formatDate, formatTimeAgo, formatBytes, formatDifficulty } from '@/utils/formatters'
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

  try {
    const blockData = await blockService.getBlockDetail(identifier)
    
    blockData.txCount = blockData.tx?.length || 0
    block.value = blockData

    // Fetch transactions in parallel
    if (blockData.tx && blockData.tx.length > 0) {
      await fetchTransactions(blockData.tx)
      
      // Detect PoS vs PoW after transactions are loaded
      if (transactions.value.length > 0) {
        const firstTx = transactions.value[0]
        blockData.isPoS = !(firstTx.vin?.[0]?.coinbase)
      } else {
        blockData.isPoS = false
      }
    }
  } catch (err) {
    console.error('Failed to fetch block:', err)
    error.value = err.message || 'Failed to load block'
  } finally {
    loading.value = false
  }
}

const fetchTransactions = async (txids) => {
  loadingTransactions.value = true

  try {
    const txPromises = txids.map(txid => transactionService.getTransaction(txid))
    const results = await Promise.allSettled(txPromises)
    
    transactions.value = results
      .filter(result => result.status === 'fulfilled')
      .map(result => result.value)
  } catch (err) {
    console.error('Failed to fetch transactions:', err)
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
    console.log('🔄 Reorg detected - refetching block data')
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
