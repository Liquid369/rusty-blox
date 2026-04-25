<template>
  <AppLayout>
    <div class="block-detail-page">
      <div v-if="loading" class="skeleton" style="height: 400px;"></div>

      <div v-else-if="block">
        <h1>Block #{{ block.height.toLocaleString() }}</h1>
        <span v-if="block.confirmations" class="badge badge-success">
          ✓ {{ block.confirmations.toLocaleString() }} Confirmations
        </span>

        <UiCard class="mt-6">
          <template #header>
            <h2>Overview</h2>
          </template>

          <div class="detail-grid">
            <div class="detail-row">
              <span class="detail-label">Hash</span>
              <span class="mono detail-value">{{ block.hash }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Previous Block</span>
              <router-link :to="`/block/${block.height - 1}`" class="detail-value">
                {{ truncateHash(block.previousblockhash) }}
              </router-link>
            </div>
            <div class="detail-row">
              <span class="detail-label">Next Block</span>
              <router-link v-if="block.nextblockhash" :to="`/block/${block.height + 1}`" class="detail-value">
                {{ truncateHash(block.nextblockhash) }}
              </router-link>
              <span v-else class="detail-value text-tertiary">N/A</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Time</span>
              <span class="detail-value">{{ formatDate(block.time) }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Difficulty</span>
              <span class="detail-value">{{ block.difficulty?.toLocaleString() }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Size</span>
              <span class="detail-value">{{ formatSize(block.size) }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Version</span>
              <span class="detail-value">{{ block.version }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Merkle Root</span>
              <span class="mono detail-value">{{ block.merkleroot }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Nonce</span>
              <span class="detail-value">{{ block.nonce?.toLocaleString() }}</span>
            </div>
          </div>
        </UiCard>

        <div class="mt-8">
          <div class="section-header">
            <h2>Transactions ({{ block.tx?.length || 0 }})</h2>
          </div>
          <div class="tx-list mt-6">
            <TransactionCard 
              v-for="tx in block.tx" 
              :key="tx.txid" 
              :tx="tx"
              @click="goToTx"
            />
          </div>
        </div>
      </div>

      <div v-else class="error-message">
        <h2>Block not found</h2>
        <p>The requested block could not be found.</p>
        <UiButton @click="$router.push('/blocks')">View Latest Blocks</UiButton>
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { blockService } from '@/services'
import AppLayout from '@/components/layout/AppLayout.vue'
import UiCard from '@/components/common/UiCard.vue'
import UiButton from '@/components/common/UiButton.vue'
import TransactionCard from '@/components/common/TransactionCard.vue'

const route = useRoute()
const router = useRouter()

const loading = ref(true)
const block = ref(null)

const formatDate = (timestamp) => {
  return new Date(timestamp * 1000).toLocaleString()
}

const formatSize = (bytes) => {
  return `${(bytes / 1024).toFixed(2)} KB`
}

const truncateHash = (hash) => {
  if (!hash) return ''
  return `${hash.slice(0, 8)}...${hash.slice(-8)}`
}

const goToTx = (txid) => {
  router.push(`/tx/${txid}`)
}

const loadBlock = async () => {
  loading.value = true
  try {
    const data = await blockService.getBlock(route.params.id)
    block.value = data
  } catch (error) {
    console.error('Failed to load block:', error)
    block.value = null
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  loadBlock()
})
</script>

<style scoped>
.block-detail-page {
  animation: fadeIn 0.3s ease;
}

.detail-grid {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.detail-row {
  display: flex;
  justify-content: space-between;
  gap: var(--space-4);
  padding-bottom: var(--space-3);
  border-bottom: 1px solid var(--border-subtle);
}

.detail-row:last-child {
  border-bottom: none;
  padding-bottom: 0;
}

.detail-label {
  font-weight: var(--weight-bold);
  color: var(--text-secondary);
  flex-shrink: 0;
}

.detail-value {
  color: var(--text-primary);
  text-align: right;
  word-break: break-all;
}

.tx-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.tx-id {
  color: var(--text-accent);
  font-size: var(--text-sm);
}

.error-message {
  text-align: center;
  padding: var(--space-16) var(--space-6);
}

@media (max-width: 768px) {
  .detail-row {
    flex-direction: column;
  }

  .detail-value {
    text-align: left;
  }
}
</style>
