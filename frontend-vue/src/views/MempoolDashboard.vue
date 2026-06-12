<template>
  <AppLayout>
    <div class="mempool-page">
      <div class="page-header">
        <h1>Mempool</h1>
        <span class="refresh-note text-tertiary">Auto-refreshes every 10s</span>
      </div>

      <div v-if="loading" class="skeleton-list mt-6">
        <div class="skeleton" style="height: 120px;"></div>
        <div class="skeleton" style="height: 300px;"></div>
      </div>

      <div v-else-if="error" class="error-message">
        <h2>Failed to Load Mempool</h2>
        <p class="text-secondary">{{ error }}</p>
        <UiButton @click="loadMempool(true)">Try Again</UiButton>
      </div>

      <div v-else>
        <div class="stats-grid mt-6">
          <StatCard
            label="Pending Transactions"
            :value="mempool.size"
            format="number"
          />
          <StatCard
            label="Total Size"
            :value="formatBytes(mempool.bytes)"
          />
          <StatCard
            label="Memory Usage"
            :value="mempool.usage != null ? formatBytes(mempool.usage) : '—'"
          />
        </div>

        <div class="section-header mt-8">
          <h2>Pending Transactions ({{ transactions.length }})</h2>
        </div>

        <div v-if="transactions.length" class="tx-list mt-6">
          <UiCard
            v-for="tx in transactions"
            :key="tx.txid"
            hover
            clickable
            @click="toggleDetail(tx.txid)"
          >
            <div class="tx-row">
              <div class="tx-main">
                <span class="mono tx-id">{{ truncateHash(tx.txid) }}</span>
                <span class="tx-time">{{ tx.time ? formatTimeAgo(tx.time) : 'Just seen' }}</span>
              </div>
              <div class="tx-side">
                <span class="tx-size">{{ tx.size != null ? `${tx.size.toLocaleString()} bytes` : '—' }}</span>
                <span class="tx-fee">Fee: {{ tx.fee != null ? `${tx.fee.toFixed(8)} PIV` : '—' }}</span>
              </div>
            </div>

            <div v-if="expandedTxid === tx.txid" class="tx-detail" @click.stop>
              <div v-if="loadingDetail" class="skeleton" style="height: 60px;"></div>
              <template v-else-if="detail">
                <div class="detail-row">
                  <span class="detail-label">Transaction ID</span>
                  <span class="mono detail-value">{{ detail.txid }}</span>
                </div>
                <div class="detail-row">
                  <span class="detail-label">Size</span>
                  <span class="detail-value">{{ detail.size != null ? `${detail.size.toLocaleString()} bytes` : '—' }}</span>
                </div>
                <div class="detail-row">
                  <span class="detail-label">Fee</span>
                  <span class="detail-value">{{ detail.fee != null ? `${detail.fee.toFixed(8)} PIV` : '—' }}</span>
                </div>
                <div class="detail-row">
                  <span class="detail-label">First Seen</span>
                  <span class="detail-value">{{ detail.time ? formatDate(detail.time) : '—' }}</span>
                </div>
                <router-link :to="`/tx/${detail.txid}`" class="full-tx-link">
                  View full transaction →
                </router-link>
              </template>
              <p v-else class="text-tertiary detail-missing">
                Transaction is no longer in the mempool.
                <router-link :to="`/tx/${tx.txid}`" class="full-tx-link">View full transaction →</router-link>
              </p>
            </div>
          </UiCard>
        </div>

        <UiCard v-else class="mt-6">
          <p class="text-tertiary empty-text">The mempool is empty. All transactions have been confirmed.</p>
        </UiCard>
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, onMounted, onUnmounted } from 'vue'
import { mempoolService } from '@/services'
import AppLayout from '@/components/layout/AppLayout.vue'
import StatCard from '@/components/common/StatCard.vue'
import UiCard from '@/components/common/UiCard.vue'
import UiButton from '@/components/common/UiButton.vue'

const REFRESH_MS = 10000

const loading = ref(true)
const error = ref('')
const mempool = ref({ size: 0, bytes: 0, usage: null, transactions: [] })
const transactions = ref([])
const expandedTxid = ref(null)
const detail = ref(null)
const loadingDetail = ref(false)

let refreshTimer = null

const formatBytes = (bytes) => {
  if (!bytes) return '0 B'
  const sizes = ['B', 'KB', 'MB', 'GB']
  const i = Math.min(sizes.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)))
  return `${(bytes / Math.pow(1024, i)).toFixed(i === 0 ? 0 : 2)} ${sizes[i]}`
}

const formatTimeAgo = (timestamp) => {
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - timestamp)
  if (diff < 60) return `${diff}s ago`
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`
  return `${Math.floor(diff / 3600)}h ago`
}

const formatDate = (timestamp) => {
  return new Date(timestamp * 1000).toLocaleString()
}

const truncateHash = (hash) => {
  if (!hash) return ''
  return `${hash.slice(0, 12)}...${hash.slice(-12)}`
}

const loadMempool = async (showLoading = false) => {
  if (showLoading) loading.value = true
  try {
    const data = await mempoolService.getMempool()
    mempool.value = data
    // Newest first
    transactions.value = [...(data.transactions || [])].sort((a, b) => (b.time || 0) - (a.time || 0))
    error.value = ''
  } catch (err) {
    // Only surface the error when there is nothing on screen yet
    if (!transactions.value.length) {
      error.value = err.response?.data?.error?.message || 'Could not reach the mempool endpoint.'
    }
  } finally {
    loading.value = false
  }
}

const toggleDetail = async (txid) => {
  if (expandedTxid.value === txid) {
    expandedTxid.value = null
    detail.value = null
    return
  }

  expandedTxid.value = txid
  detail.value = null
  loadingDetail.value = true
  try {
    detail.value = await mempoolService.getMempoolTransaction(txid)
  } catch (err) {
    detail.value = null
  } finally {
    loadingDetail.value = false
  }
}

onMounted(() => {
  loadMempool(true)
  refreshTimer = setInterval(loadMempool, REFRESH_MS)
})

onUnmounted(() => {
  if (refreshTimer) {
    clearInterval(refreshTimer)
    refreshTimer = null
  }
})
</script>

<style scoped>
.mempool-page {
  animation: fadeIn 0.3s ease;
}

.page-header {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-4);
  flex-wrap: wrap;
}

.refresh-note {
  font-size: var(--text-sm);
}

.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
  gap: var(--space-6);
}

.section-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding-bottom: var(--space-4);
  border-bottom: 2px solid var(--border-primary);
}

.section-header h2 {
  margin: 0;
}

.tx-list,
.skeleton-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.tx-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--space-6);
}

.tx-main {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  flex: 1;
  min-width: 0;
}

.tx-id {
  font-size: var(--text-sm);
  color: var(--text-accent);
  overflow: hidden;
  text-overflow: ellipsis;
}

.tx-time {
  font-size: var(--text-sm);
  color: var(--text-tertiary);
}

.tx-side {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: var(--space-1);
  font-size: var(--text-sm);
  color: var(--text-secondary);
}

.tx-fee {
  color: var(--text-tertiary);
}

.tx-detail {
  margin-top: var(--space-4);
  padding-top: var(--space-4);
  border-top: 1px solid var(--border-subtle);
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  cursor: default;
}

.detail-row {
  display: flex;
  justify-content: space-between;
  gap: var(--space-4);
}

.detail-label {
  font-weight: var(--weight-bold);
  color: var(--text-secondary);
  font-size: var(--text-sm);
  flex-shrink: 0;
}

.detail-value {
  color: var(--text-primary);
  font-size: var(--text-sm);
  text-align: right;
  word-break: break-all;
}

.full-tx-link {
  color: var(--text-accent);
  text-decoration: none;
  font-size: var(--text-sm);
  font-weight: var(--weight-bold);
}

.full-tx-link:hover {
  text-decoration: underline;
}

.detail-missing {
  font-size: var(--text-sm);
  margin: 0;
}

.empty-text {
  text-align: center;
  padding: var(--space-6);
  margin: 0;
}

.error-message {
  text-align: center;
  padding: var(--space-16) var(--space-6);
}

.error-message p {
  margin-bottom: var(--space-6);
}

@media (max-width: 768px) {
  .stats-grid {
    grid-template-columns: 1fr;
  }

  .tx-row {
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-3);
  }

  .tx-side {
    align-items: flex-start;
  }
}
</style>
