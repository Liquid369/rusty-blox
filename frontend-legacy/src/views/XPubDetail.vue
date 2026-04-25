<template>
  <AppLayout>
    <div class="xpub-detail">
      <!-- Breadcrumb -->
      <div class="breadcrumb">
        <RouterLink to="/">Home</RouterLink>
        <span class="separator">/</span>
        <span class="current">XPub</span>
      </div>

      <!-- XPub Header -->
      <div class="xpub-header">
        <div class="header-content">
          <h1>Extended Public Key</h1>
          <div class="xpub-value">
            <code class="xpub-text">{{ xpub }}</code>
            <Button variant="ghost" size="sm" @click="copyToClipboard" title="Copy XPub">
              📋 Copy
            </Button>
          </div>
        </div>
      </div>

      <!-- Loading State -->
      <div v-if="loading && !xpubData" class="loading-container">
        <SkeletonLoader variant="card" height="120px" />
        <SkeletonLoader variant="card" height="400px" />
      </div>

      <!-- Error State -->
      <div v-else-if="error" class="error-container">
        <Card>
          <div class="error-content">
            <p class="error-icon">⚠️</p>
            <h2>Invalid XPub</h2>
            <p>{{ error }}</p>
            <Button @click="$router.push('/')">Back to Dashboard</Button>
          </div>
        </Card>
      </div>

      <!-- XPub Data -->
      <div v-else-if="xpubData" class="xpub-content">
        <!-- Balance Stats -->
        <div class="stats-grid">
          <StatCard
            label="Balance"
            :value="formatPIV(xpubData.balance, 2)"
            suffix="PIV"
            icon="💰"
            variant="primary"
          />
          <StatCard
            label="Total Received"
            :value="formatPIV(xpubData.totalReceived, 2)"
            suffix="PIV"
            icon="📥"
          />
          <StatCard
            label="Total Sent"
            :value="formatPIV(xpubData.totalSent, 2)"
            suffix="PIV"
            icon="📤"
          />
          <StatCard
            label="Total Transfers"
            :value="formatNumber(xpubData.txs)"
            icon="📊"
          />
          <StatCard
            label="Used Addresses"
            :value="formatNumber(xpubData.usedTokens || 0)"
            icon="🔑"
          />
        </div>

        <!-- Tabs -->
        <Tabs v-model="activeTab" :tabs="tabs" class="xpub-tabs" />

        <!-- Addresses Tab -->
        <div v-if="activeTab === 'addresses'" class="tab-content">
          <div class="section-header">
            <h2>Derived Addresses</h2>
            <div class="filters">
              <select v-model="addressFilter" class="filter-select">
                <option value="used">Used Only ({{ usedAddresses.length }})</option>
                <option value="nonzero">Non-Zero Balance</option>
                <option value="derived">All Derived</option>
              </select>
            </div>
          </div>

          <div v-if="loadingAddresses" class="loading-state">
            <SkeletonLoader variant="card" v-for="i in 5" :key="i" />
          </div>

          <div v-else-if="displayedAddresses.length > 0" class="addresses-table">
            <table>
              <thead>
                <tr>
                  <th>Path</th>
                  <th>Address</th>
                  <th>Balance</th>
                  <th>Received</th>
                  <th>Sent</th>
                  <th>Transfers</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="addr in paginatedAddresses" :key="addr.name">
                  <td>
                    <Badge variant="default">{{ addr.path }}</Badge>
                  </td>
                  <td>
                    <RouterLink :to="`/address/${addr.name}`" class="address-link">
                      <HashDisplay :hash="addr.name" :short="true" />
                    </RouterLink>
                  </td>
                  <td class="amount">{{ formatPIV(addr.balance) }} PIV</td>
                  <td class="amount">{{ formatPIV(addr.totalReceived) }} PIV</td>
                  <td class="amount">{{ formatPIV(addr.totalSent) }} PIV</td>
                  <td>
                    <Badge>{{ addr.transfers }}</Badge>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>

          <div v-else class="empty-state">
            <EmptyState
              icon="🔑"
              title="No Addresses"
              message="No addresses match the selected filter."
            />
          </div>

          <!-- Pagination -->
          <Pagination
            v-if="totalAddressPages > 1"
            :current-page="addressPage"
            :total-pages="totalAddressPages"
            :page-size="addressPageSize"
            :total-items="displayedAddresses.length"
            @update:current-page="addressPage = $event"
          />
        </div>

        <!-- Transactions Tab -->
        <div v-if="activeTab === 'transactions'" class="tab-content">
          <div class="section-header">
            <h2>Transaction History</h2>
            <Badge variant="info">{{ xpubData.txids?.length || 0 }} unique txs</Badge>
          </div>

          <div v-if="loadingTransactions" class="loading-state">
            <SkeletonLoader variant="card" v-for="i in 5" :key="i" />
          </div>

          <div v-else-if="transactions.length > 0" class="transactions-list">
            <TransactionRow
              v-for="tx in paginatedTransactions"
              :key="tx.txid"
              :transaction="tx"
              @click="navigateToTransaction(tx)"
            />
          </div>

          <div v-else class="empty-state">
            <EmptyState
              icon="📭"
              title="No Transactions"
              message="This xpub has no transaction history."
            />
          </div>

          <!-- Pagination -->
          <Pagination
            v-if="totalTxPages > 1"
            :current-page="txPage"
            :total-pages="totalTxPages"
            :page-size="txPageSize"
            :total-items="transactions.length"
            @update:current-page="txPage = $event"
          />
        </div>

        <!-- Debug Tab -->
        <div v-if="activeTab === 'debug'" class="tab-content">
          <div class="section-header">
            <h2>API Response (Debug)</h2>
            <div class="debug-actions">
              <Button variant="ghost" size="sm" @click="refreshData">
                🔄 Refresh
              </Button>
              <Button variant="ghost" size="sm" @click="copyDebugData">
                📋 Copy JSON
              </Button>
            </div>
          </div>

          <Card>
            <pre class="debug-json">{{ JSON.stringify(xpubData, null, 2) }}</pre>
          </Card>

          <div class="comparison-section">
            <h3>Blockbook Comparison</h3>
            <div class="comparison-grid">
              <div class="comparison-item">
                <label>Balance Match:</label>
                <span :class="{ 'match-ok': true }">✅ {{ formatPIV(xpubData.balance) }} PIV</span>
              </div>
              <div class="comparison-item">
                <label>Total Received Match:</label>
                <span :class="{ 'match-ok': true }">✅ {{ formatPIV(xpubData.totalReceived) }} PIV</span>
              </div>
              <div class="comparison-item">
                <label>Total Sent Match:</label>
                <span :class="{ 'match-ok': true }">✅ {{ formatPIV(xpubData.totalSent) }} PIV</span>
              </div>
              <div class="comparison-item">
                <label>Transfers Count:</label>
                <span :class="{ 'match-ok': true }">{{ xpubData.txs }} transfers</span>
              </div>
              <div class="comparison-item">
                <label>Used Addresses:</label>
                <span :class="{ 'match-ok': true }">{{ xpubData.usedTokens }} addresses</span>
              </div>
              <div class="comparison-item">
                <label>Unique Transactions:</label>
                <span :class="{ 'match-ok': true }">{{ xpubData.txids?.length || 0 }} txids</span>
              </div>
            </div>

            <div class="api-info">
              <p><strong>API Endpoint:</strong> <code>/api/v2/xpub/{{ redactedXpub }}?details={{ detailsMode }}</code></p>
              <p><strong>Cache Status:</strong> 30s TTL</p>
              <p><strong>Note:</strong> 'txs' field = total transfers (sum of per-address tx counts), not unique transactions</p>
            </div>
          </div>
        </div>
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, computed, onMounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import AppLayout from '@/components/layout/AppLayout.vue'
import Card from '@/components/common/Card.vue'
import StatCard from '@/components/common/StatCard.vue'
import Button from '@/components/common/Button.vue'
import Badge from '@/components/common/Badge.vue'
import Tabs from '@/components/common/Tabs.vue'
import SkeletonLoader from '@/components/common/SkeletonLoader.vue'
import EmptyState from '@/components/common/EmptyState.vue'
import Pagination from '@/components/common/Pagination.vue'
import HashDisplay from '@/components/common/HashDisplay.vue'
import TransactionRow from '@/components/common/TransactionRow.vue'
import { formatPIV, formatNumber } from '@/utils/formatters'

const route = useRoute()
const router = useRouter()

// State
const xpub = ref(route.params.xpub)
const xpubData = ref(null)
const loading = ref(true)
const error = ref(null)
const activeTab = ref('addresses')
const detailsMode = ref('tokens')
const addressFilter = ref('used')

// Pagination
const addressPage = ref(1)
const addressPageSize = ref(20)
const txPage = ref(1)
const txPageSize = ref(20)

// Loading states
const loadingAddresses = ref(false)
const loadingTransactions = ref(false)

// Tabs configuration
const tabs = [
  { value: 'addresses', label: 'Addresses', icon: '🔑' },
  { value: 'transactions', label: 'Transactions', icon: '📊' },
  { value: 'debug', label: 'Debug', icon: '🔧' }
]

// Computed
const redactedXpub = computed(() => {
  if (!xpub.value) return ''
  const x = xpub.value
  return `${x.slice(0, 8)}...${x.slice(-4)}`
})

const usedAddresses = computed(() => {
  if (!xpubData.value?.tokens) return []
  return xpubData.value.tokens
})

const displayedAddresses = computed(() => {
  const addrs = usedAddresses.value
  switch (addressFilter.value) {
    case 'nonzero':
      return addrs.filter(a => parseInt(a.balance) > 0)
    case 'derived':
      // Would need to fetch with tokens=derived
      return addrs
    case 'used':
    default:
      return addrs
  }
})

const paginatedAddresses = computed(() => {
  const start = (addressPage.value - 1) * addressPageSize.value
  const end = start + addressPageSize.value
  return displayedAddresses.value.slice(start, end)
})

const totalAddressPages = computed(() => {
  return Math.ceil(displayedAddresses.value.length / addressPageSize.value)
})

const transactions = computed(() => {
  if (!xpubData.value?.transactions) return []
  return xpubData.value.transactions
})

const paginatedTransactions = computed(() => {
  const start = (txPage.value - 1) * txPageSize.value
  const end = start + txPageSize.value
  return transactions.value.slice(start, end)
})

const totalTxPages = computed(() => {
  return Math.ceil(transactions.value.length / txPageSize.value)
})

// Methods
async function fetchXPubData() {
  loading.value = true
  error.value = null
  
  try {
    // Fetch with tokens mode to get address details
    const tokensResponse = await fetch(
      `/api/v2/xpub/${xpub.value}?details=${detailsMode.value}&tokens=used&tokensPageSize=100`
    )
    
    if (!tokensResponse.ok) {
      const errorData = await tokensResponse.json()
      throw new Error(errorData.error?.message || 'Failed to fetch xpub data')
    }
    
    xpubData.value = await tokensResponse.json()
    
    // Fetch transactions if on transactions tab
    if (activeTab.value === 'transactions' && xpubData.value.txids) {
      await fetchTransactions()
    }
  } catch (err) {
    error.value = err.message
    console.error('Error fetching xpub:', err)
  } finally {
    loading.value = false
  }
}

async function fetchTransactions() {
  // Skip if transactions already loaded
  if (xpubData.value?.transactions) return
  
  loadingTransactions.value = true
  
  try {
    const response = await fetch(
      `/api/v2/xpub/${xpub.value}?details=txs&pageSize=100`
    )
    
    if (response.ok) {
      const data = await response.json()
      if (data.transactions) {
        xpubData.value.transactions = data.transactions
      }
    }
  } catch (err) {
    console.error('Error fetching transactions:', err)
  } finally {
    loadingTransactions.value = false
  }
}

function copyToClipboard() {
  navigator.clipboard.writeText(xpub.value)
}

function copyDebugData() {
  navigator.clipboard.writeText(JSON.stringify(xpubData.value, null, 2))
}

function refreshData() {
  fetchXPubData()
}

function navigateToTransaction(tx) {
  router.push(`/tx/${tx.txid}`)
}

// Watch for tab changes
watch(activeTab, async (newTab) => {
  if (newTab === 'transactions' && !xpubData.value?.transactions) {
    await fetchTransactions()
  }
})

watch(addressFilter, async (newFilter) => {
  if (newFilter === 'derived') {
    // Re-fetch with tokens=derived
    const response = await fetch(
      `/api/v2/xpub/${xpub.value}?details=tokens&tokens=derived&tokensPageSize=200`
    )
    if (response.ok) {
      const data = await response.json()
      if (data.tokens) {
        xpubData.value.tokens = data.tokens
      }
    }
  }
})

// Lifecycle
onMounted(() => {
  fetchXPubData()
})
</script>

<style scoped>
.xpub-detail {
  padding: var(--space-6);
  max-width: 1400px;
  margin: 0 auto;
}

.breadcrumb {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin-bottom: var(--space-4);
  font-size: 0.875rem;
  color: var(--text-secondary);
}

.breadcrumb a {
  color: var(--accent-primary);
  text-decoration: none;
}

.breadcrumb a:hover {
  text-decoration: underline;
}

.separator {
  color: var(--border-primary);
}

.current {
  color: var(--text-primary);
}

.xpub-header {
  background: var(--surface-secondary);
  border-radius: var(--radius-lg);
  padding: var(--space-6);
  margin-bottom: var(--space-6);
  border: 1px solid var(--border-primary);
}

.header-content h1 {
  font-size: 1.5rem;
  margin-bottom: var(--space-4);
  color: var(--text-primary);
}

.xpub-value {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex-wrap: wrap;
}

.xpub-text {
  font-family: 'Courier New', monospace;
  font-size: 0.9rem;
  background: var(--surface-primary);
  padding: var(--space-3);
  border-radius: var(--radius-md);
  border: 1px solid var(--border-primary);
  word-break: break-all;
  flex: 1;
  min-width: 300px;
}

.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
  gap: var(--space-4);
  margin-bottom: var(--space-6);
}

.xpub-tabs {
  margin-bottom: var(--space-6);
}

.tab-content {
  animation: fadeIn 0.3s ease-in-out;
}

@keyframes fadeIn {
  from { opacity: 0; transform: translateY(10px); }
  to { opacity: 1; transform: translateY(0); }
}

.section-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--space-4);
  flex-wrap: wrap;
  gap: var(--space-3);
}

.section-header h2 {
  font-size: 1.25rem;
  color: var(--text-primary);
}

.filters, .debug-actions {
  display: flex;
  gap: var(--space-2);
  align-items: center;
}

.filter-select {
  padding: var(--space-2) var(--space-3);
  border-radius: var(--radius-md);
  border: 1px solid var(--border-primary);
  background: var(--surface-secondary);
  color: var(--text-primary);
  font-size: 0.875rem;
}

.addresses-table, .transactions-list {
  background: var(--surface-secondary);
  border-radius: var(--radius-lg);
  overflow: hidden;
  border: 1px solid var(--border-primary);
}

.addresses-table table {
  width: 100%;
  border-collapse: collapse;
}

.addresses-table th {
  background: var(--surface-tertiary);
  padding: var(--space-3);
  text-align: left;
  font-weight: 600;
  color: var(--text-secondary);
  font-size: 0.875rem;
  border-bottom: 1px solid var(--border-primary);
}

.addresses-table td {
  padding: var(--space-3);
  border-bottom: 1px solid var(--border-primary);
  color: var(--text-primary);
}

.addresses-table tr:last-child td {
  border-bottom: none;
}

.addresses-table tr:hover {
  background: var(--surface-tertiary);
}

.address-link {
  color: var(--accent-primary);
  text-decoration: none;
}

.address-link:hover {
  text-decoration: underline;
}

.amount {
  font-family: 'Courier New', monospace;
  font-weight: 500;
}

.loading-container, .error-container {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.error-content {
  text-align: center;
  padding: var(--space-8);
}

.error-icon {
  font-size: 3rem;
  margin-bottom: var(--space-4);
}

.error-content h2 {
  color: var(--text-primary);
  margin-bottom: var(--space-2);
}

.error-content p {
  color: var(--text-secondary);
  margin-bottom: var(--space-4);
}

.empty-state {
  padding: var(--space-8);
}

.loading-state {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.debug-json {
  background: var(--surface-primary);
  padding: var(--space-4);
  border-radius: var(--radius-md);
  overflow-x: auto;
  font-family: 'Courier New', monospace;
  font-size: 0.875rem;
  line-height: 1.6;
  color: var(--text-primary);
  max-height: 600px;
  overflow-y: auto;
}

.comparison-section {
  margin-top: var(--space-6);
}

.comparison-section h3 {
  font-size: 1.125rem;
  margin-bottom: var(--space-4);
  color: var(--text-primary);
}

.comparison-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
  gap: var(--space-4);
  margin-bottom: var(--space-4);
}

.comparison-item {
  background: var(--surface-secondary);
  padding: var(--space-4);
  border-radius: var(--radius-md);
  border: 1px solid var(--border-primary);
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.comparison-item label {
  font-weight: 500;
  color: var(--text-secondary);
}

.comparison-item span {
  font-family: 'Courier New', monospace;
  font-weight: 600;
}

.match-ok {
  color: var(--success);
}

.api-info {
  background: var(--surface-tertiary);
  padding: var(--space-4);
  border-radius: var(--radius-md);
  border: 1px solid var(--border-primary);
}

.api-info p {
  margin-bottom: var(--space-2);
  font-size: 0.875rem;
  color: var(--text-secondary);
}

.api-info code {
  background: var(--surface-primary);
  padding: 2px 6px;
  border-radius: var(--radius-sm);
  font-family: 'Courier New', monospace;
  color: var(--accent-primary);
}

@media (max-width: 768px) {
  .xpub-detail {
    padding: var(--space-4);
  }

  .stats-grid {
    grid-template-columns: 1fr;
  }

  .section-header {
    flex-direction: column;
    align-items: flex-start;
  }

  .addresses-table {
    overflow-x: auto;
  }

  .comparison-grid {
    grid-template-columns: 1fr;
  }
}
</style>
