<template>
  <AppLayout>
    <div class="mempool-page">
      <div class="page-header">
        <div class="header-row">
          <h1>Mempool</h1>
          <LiveIndicator :connected="!error" show-label />
        </div>
        <p class="page-subtitle">Unconfirmed transactions waiting for the next block</p>
      </div>

      <!-- Loading State -->
      <div v-if="loading && !mempool" class="loading-container">
        <LoadingSpinner size="lg" text="Loading mempool..." />
      </div>

      <!-- Error State -->
      <div v-else-if="error" class="error-container">
        <EmptyState
          icon="alert-triangle"
          title="Failed to Load Mempool"
          :message="error"
        >
          <template #action>
            <Button @click="fetchMempool">Try Again</Button>
          </template>
        </EmptyState>
      </div>

      <div v-else-if="mempool">
        <!-- Summary Band -->
        <div class="stats-grid">
          <StatCard
            label="Pending Transactions"
            :value="formatNumber(mempool.size)"
            icon="hourglass"
          />
          <StatCard
            label="Mempool Size"
            :value="formatBytes(mempool.bytes || 0)"
            icon="database"
          />
          <StatCard
            label="Total Fees"
            :value="totalFees"
            suffix="PIV"
            icon="settings"
          />
          <StatCard
            label="Oldest Transaction"
            :value="oldestAge"
            icon="clock"
          />
        </div>

        <!-- Transaction List -->
        <Card class="mempool-card">
          <template #header>
            <div class="card-header-row">
              <span>Pending Transactions</span>
              <Badge variant="info">{{ rows.length }} shown</Badge>
            </div>
          </template>

          <EmptyState
            v-if="rows.length === 0"
            icon="inbox"
            title="Mempool Is Empty"
            message="All transactions have been confirmed. New transactions will appear here automatically."
          />

          <div v-else class="mempool-table-container">
            <table class="mempool-table">
              <thead>
                <tr>
                  <th>Transaction</th>
                  <th>Type</th>
                  <th class="num">Amount</th>
                  <th class="num">Size</th>
                  <th class="num">Fee</th>
                  <th class="num">Fee Rate</th>
                  <th class="num">Age</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="row in rows" :key="row.txid">
                  <td>
                    <HashDisplay
                      :hash="row.txid"
                      :start-length="10"
                      :end-length="10"
                      show-copy
                      :link-to="`/tx/${row.txid}`"
                    />
                  </td>
                  <td>
                    <Badge :variant="row.typeVariant" size="sm">{{ row.typeLabel }}</Badge>
                  </td>
                  <td class="num amount">{{ row.amount !== null ? `${row.amount} PIV` : '—' }}</td>
                  <td class="num">{{ row.size ? formatBytes(row.size) : '—' }}</td>
                  <td class="num">{{ row.fee !== null ? `${row.fee} PIV` : '—' }}</td>
                  <td class="num">{{ row.feeRate !== null ? `${row.feeRate} sat/B` : '—' }}</td>
                  <td class="num">{{ row.time ? formatTimeAgo(row.time) : '—' }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </Card>
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue'
import api from '@/services/api'
import { transactionService } from '@/services/transactionService'
import {
  detectTransactionType,
  getTransactionTypeLabel,
  getTransactionTypeBadgeVariant
} from '@/utils/transactionHelpers'
import { formatNumber, formatBytes, formatTimeAgo, formatPIV, formatDuration } from '@/utils/formatters'
import AppLayout from '@/components/layout/AppLayout.vue'
import Card from '@/components/common/Card.vue'
import Badge from '@/components/common/Badge.vue'
import Button from '@/components/common/Button.vue'
import StatCard from '@/components/common/StatCard.vue'
import HashDisplay from '@/components/common/HashDisplay.vue'
import LiveIndicator from '@/components/common/LiveIndicator.vue'
import LoadingSpinner from '@/components/common/LoadingSpinner.vue'
import EmptyState from '@/components/common/EmptyState.vue'

const REFRESH_INTERVAL_MS = 15000
const MAX_DETAILED_TXS = 50

const mempool = ref(null)
const details = ref({}) // txid -> full tx (when resolvable)
const loading = ref(false)
const error = ref('')
let refreshTimer = null

// Mempool fee values arrive as PIV floats from the node
const totalFees = computed(() => {
  if (!mempool.value?.transactions?.length) return '0'
  const total = mempool.value.transactions.reduce((sum, tx) => sum + (tx.fee || 0), 0)
  return total.toFixed(4)
})

const oldestAge = computed(() => {
  const times = (mempool.value?.transactions || [])
    .map(tx => tx.time)
    .filter(Boolean)
  if (!times.length) return '—'
  const oldest = Math.min(...times)
  const age = Math.max(0, Math.floor(Date.now() / 1000) - oldest)
  return formatDuration(age)
})

const rows = computed(() => {
  const txs = [...(mempool.value?.transactions || [])]
  txs.sort((a, b) => (b.time || 0) - (a.time || 0))

  return txs.map(entry => {
    const detail = details.value[entry.txid]
    const type = detail ? detectTransactionType(detail) : null
    const feeSats = entry.fee ? entry.fee * 100000000 : 0
    const size = entry.size || detail?.size || 0
    return {
      txid: entry.txid,
      typeLabel: type ? getTransactionTypeLabel(type) : 'Pending',
      typeVariant: type ? getTransactionTypeBadgeVariant(type) : 'default',
      amount: detail ? formatPIV(detail.value) : null,
      size,
      fee: entry.fee !== null && entry.fee !== undefined ? entry.fee.toFixed(8) : null,
      feeRate: feeSats && size ? (feeSats / size).toFixed(2) : null,
      time: entry.time || null
    }
  })
})

const fetchMempool = async () => {
  loading.value = true
  error.value = ''

  try {
    const response = await api.get('/api/v2/mempool')
    mempool.value = response.data
    await fetchDetails(response.data?.transactions || [])
  } catch (err) {
    error.value = err.message || 'Failed to load mempool'
  } finally {
    loading.value = false
  }
}

// Resolve full tx objects (best effort) so rows can be type-tagged
const fetchDetails = async (entries) => {
  const pending = entries
    .slice(0, MAX_DETAILED_TXS)
    .filter(entry => !details.value[entry.txid])

  if (!pending.length) return

  const results = await Promise.allSettled(
    pending.map(entry => transactionService.getTransaction(entry.txid))
  )

  const next = { ...details.value }
  results.forEach((result, idx) => {
    if (result.status === 'fulfilled') {
      next[pending[idx].txid] = result.value
    }
  })
  details.value = next
}

const refresh = async () => {
  try {
    const response = await api.get('/api/v2/mempool')
    mempool.value = response.data
    error.value = ''
    await fetchDetails(response.data?.transactions || [])
  } catch {
    // keep showing the last snapshot on transient refresh errors
  }
}

onMounted(() => {
  fetchMempool()
  refreshTimer = setInterval(refresh, REFRESH_INTERVAL_MS)
})

onUnmounted(() => {
  if (refreshTimer) clearInterval(refreshTimer)
})
</script>

<style scoped>
.mempool-page {
  padding: var(--space-6);
  max-width: 1400px;
  margin: 0 auto;
}

.page-header {
  margin-bottom: var(--space-6);
}

.header-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

.page-subtitle {
  color: var(--text-secondary);
  font-size: var(--text-lg);
  margin-top: var(--space-2);
}

/* 4 tiles: keep rows balanced (4 / 2x2 / 1) instead of wrapping 3+1 */
.stats-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: var(--space-4);
  margin-bottom: var(--space-6);
}

@media (max-width: 1024px) {
  .stats-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

@media (max-width: 520px) {
  .stats-grid {
    grid-template-columns: 1fr;
  }
}

.card-header-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
}

.mempool-table-container {
  overflow-x: auto;
}

.mempool-table {
  width: 100%;
  border-collapse: collapse;
}

.mempool-table th {
  padding: var(--space-3) var(--space-4);
  text-align: left;
  font-size: var(--text-xs);
  font-weight: var(--weight-semibold);
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
  background: rgba(var(--rgb-purple-darkest), 0.92);
  border-bottom: 1px solid var(--border-primary);
}

.mempool-table td {
  padding: var(--space-3) var(--space-4);
  border-top: 1px solid var(--border-subtle);
  font-size: var(--text-sm);
  font-variant-numeric: tabular-nums;
}

.mempool-table .num {
  text-align: right;
  font-family: var(--font-mono);
}

.mempool-table tbody tr {
  transition: background-color var(--transition-fast);
}

.mempool-table tbody tr:hover {
  background: var(--bg-hover);
}

.amount {
  font-weight: var(--weight-bold);
  color: var(--text-accent);
}

.loading-container,
.error-container {
  min-height: 400px;
  display: flex;
  align-items: center;
  justify-content: center;
}

@media (max-width: 768px) {
  .mempool-page {
    padding: var(--space-4);
  }
}
</style>
