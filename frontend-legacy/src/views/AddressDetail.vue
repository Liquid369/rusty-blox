<template>
  <AppLayout>
    <div class="address-detail">
      <!-- Breadcrumb -->
      <div class="breadcrumb">
        <RouterLink to="/">Home</RouterLink>
        <span class="separator">/</span>
        <span class="current">Address</span>
      </div>

      <!-- Address Header -->
      <div class="address-header">
        <div class="header-content">
          <h1>Address</h1>
          <div class="address-value">
            <HashDisplay :hash="address" :copyable="true" :linkable="false" />
            <Button variant="ghost" size="sm" @click="showQR = true" title="Show QR Code">
              📱 QR
            </Button>
          </div>
        </div>
      </div>

      <!-- Loading State -->
      <div v-if="loading && !addressData" class="loading-container">
        <SkeletonLoader variant="card" height="120px" />
        <SkeletonLoader variant="card" height="400px" />
      </div>

      <!-- Error State -->
      <div v-else-if="error" class="error-container">
        <Card>
          <div class="error-content">
            <p class="error-icon">⚠️</p>
            <h2>Address Not Found</h2>
            <p>{{ error }}</p>
            <Button @click="$router.push('/')">Back to Dashboard</Button>
          </div>
        </Card>
      </div>

      <!-- Address Data -->
      <div v-else-if="addressData" class="address-content">
        <!-- Balance Stats -->
        <div class="stats-grid">
          <StatCard
            label="Balance"
            :value="formatPIV(addressData.balance)"
            suffix="PIV"
            icon="💰"
            variant="primary"
          />
          <StatCard
            label="Total Received"
            :value="formatPIV(addressData.totalReceived)"
            suffix="PIV"
            icon="📥"
          />
          <StatCard
            label="Total Sent"
            :value="formatPIV(addressData.totalSent)"
            suffix="PIV"
            icon="📤"
          />
          <StatCard
            label="Transactions"
            :value="formatNumber(addressData.txids?.length || 0)"
            icon="📊"
          />
        </div>

        <!-- Tabs -->
        <Tabs v-model="activeTab" :tabs="tabs" class="address-tabs" />

        <!-- Transactions Tab -->
        <div v-if="activeTab === 'transactions'" class="tab-content">
          <div class="section-header">
            <h2>Transaction History</h2>
            <div class="filters">
              <select v-model="txFilter" class="filter-select">
                <option value="all">All Transactions</option>
                <option value="received">Received Only</option>
                <option value="sent">Sent Only</option>
              </select>
            </div>
          </div>

          <div v-if="filteredTransactions.length > 0" class="transactions-list">
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
              message="This address has no transaction history."
            />
          </div>

          <!-- Pagination -->
          <Pagination
            v-if="totalTxPages > 1"
            :current-page="txPage"
            :total-pages="totalTxPages"
            :page-size="txPageSize"
            :total-items="filteredTransactions.length"
            @update:current-page="txPage = $event"
          />
        </div>

        <!-- UTXOs Tab -->
        <div v-if="activeTab === 'utxos'" class="tab-content">
          <div class="section-header">
            <h2>Unspent Outputs (UTXOs)</h2>
            <Badge variant="info">{{ utxos.length }} UTXOs</Badge>
          </div>

          <div v-if="loadingUtxos" class="loading-state">
            <SkeletonLoader variant="card" v-for="i in 5" :key="i" />
          </div>

          <div v-else-if="utxos.length > 0" class="utxos-table">
            <table>
              <thead>
                <tr>
                  <th>Transaction</th>
                  <th>Output</th>
                  <th>Amount</th>
                  <th>Confirmations</th>
                  <th>Height</th>
                  <th>Status</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="utxo in paginatedUtxos" :key="`${utxo.txid}:${utxo.vout}`">
                  <td>
                    <HashDisplay :hash="utxo.txid" :short="true" :copyable="true" />
                  </td>
                  <td>
                    <Badge variant="secondary">{{ utxo.vout }}</Badge>
                  </td>
                  <td class="amount">{{ formatPIV(utxo.value) }} PIV</td>
                  <td>{{ formatNumber(utxo.confirmations) }}</td>
                  <td>
                    <RouterLink :to="`/block/${utxo.height}`" class="height-link">
                      {{ formatNumber(utxo.height) }}
                    </RouterLink>
                  </td>
                  <td>
                    <div class="status-badges">
                      <Badge :variant="utxo.actuallySpendable ? 'success' : 'warning'">
                        {{ utxo.actuallySpendable ? 'Spendable' : (utxo.maturityInfo ? 'Maturing' : 'Locked') }}
                      </Badge>
                      <Badge v-if="utxo.coinbase" variant="info" class="ml-2">Coinbase</Badge>
                      <Badge v-if="utxo.coinstake" variant="info" class="ml-2">Coinstake</Badge>
                      <Badge v-if="utxo.maturityInfo" variant="warning" class="ml-2" :title="`Requires ${utxo.maturityInfo.required} confirmations`">
                        {{ utxo.maturityInfo.current }}/{{ utxo.maturityInfo.required }}
                      </Badge>
                    </div>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>

          <div v-else class="empty-state">
            <EmptyState
              icon="💸"
              title="No UTXOs"
              message="This address has no unspent outputs."
            />
          </div>

          <!-- UTXO Pagination -->
          <Pagination
            v-if="totalUtxoPages > 1"
            :current-page="utxoPage"
            :total-pages="totalUtxoPages"
            :page-size="utxoPageSize"
            :total-items="utxos.length"
            @update:current-page="utxoPage = $event"
          />
        </div>

        <!-- QR Code Tab -->
        <div v-if="activeTab === 'qr'" class="tab-content">
          <Card class="qr-card">
            <div class="qr-content">
              <h2>Address QR Code</h2>
              <div class="qr-code-container">
                <canvas ref="qrCanvas" class="qr-canvas"></canvas>
              </div>
              <p class="qr-address">{{ address }}</p>
              <div class="qr-actions">
                <Button @click="downloadQR">💾 Download PNG</Button>
                <Button variant="secondary" @click="copyAddress">📋 Copy Address</Button>
              </div>
            </div>
          </Card>
        </div>
      </div>

      <!-- QR Modal -->
      <Modal v-model="showQR" title="Address QR Code">
        <div class="qr-modal-content">
          <canvas ref="qrModalCanvas" class="qr-canvas"></canvas>
          <p class="qr-address">{{ address }}</p>
          <Button @click="downloadQR" class="download-btn">💾 Download</Button>
        </div>
      </Modal>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, computed, watch, onMounted, nextTick } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { addressService } from '@/services/addressService'
import { transactionService } from '@/services/transactionService'
import { formatPIV, formatNumber } from '@/utils/formatters'
import { detectTransactionType } from '@/utils/transactionHelpers'
import AppLayout from '@/components/layout/AppLayout.vue'
import Card from '@/components/common/Card.vue'
import StatCard from '@/components/common/StatCard.vue'
import Tabs from '@/components/common/Tabs.vue'
import Badge from '@/components/common/Badge.vue'
import Button from '@/components/common/Button.vue'
import HashDisplay from '@/components/common/HashDisplay.vue'
import TransactionRow from '@/components/common/TransactionRow.vue'
import Pagination from '@/components/common/Pagination.vue'
import SkeletonLoader from '@/components/common/SkeletonLoader.vue'
import EmptyState from '@/components/common/EmptyState.vue'
import Modal from '@/components/common/Modal.vue'
import QRCode from 'qrcode'

const route = useRoute()
const router = useRouter()

const address = ref(route.params.address)
const addressData = ref(null)
const transactions = ref([])
const utxos = ref([])
const loading = ref(false)
const loadingUtxos = ref(false)
const error = ref(null)

// Tabs
const activeTab = ref('transactions')
const tabs = [
  { value: 'transactions', label: 'Transactions' },
  { value: 'utxos', label: 'UTXOs' },
  { value: 'qr', label: 'QR Code' }
]

// Transaction filtering and pagination
const txFilter = ref('all')
const txPage = ref(1)
const txPageSize = ref(25)

// UTXO pagination
const utxoPage = ref(1)
const utxoPageSize = ref(25)

// QR Code
const showQR = ref(false)
const qrCanvas = ref(null)
const qrModalCanvas = ref(null)

// Computed
const filteredTransactions = computed(() => {
  if (txFilter.value === 'all') return transactions.value
  
  return transactions.value.filter(tx => {
    // Determine if this address received or sent based on inputs/outputs
    const isReceived = tx.vout?.some(output => 
      output.scriptPubKey?.addresses?.includes(address.value)
    )
    const isSent = tx.vin?.some(input => 
      input.addresses?.includes(address.value)
    )
    
    if (txFilter.value === 'received') return isReceived && !isSent
    if (txFilter.value === 'sent') return isSent
    return true
  })
})

const totalTxPages = computed(() => 
  Math.ceil(filteredTransactions.value.length / txPageSize.value)
)

const paginatedTransactions = computed(() => {
  const start = (txPage.value - 1) * txPageSize.value
  const end = start + txPageSize.value
  return filteredTransactions.value.slice(start, end)
})

const totalUtxoPages = computed(() => 
  Math.ceil(utxos.value.length / utxoPageSize.value)
)

const paginatedUtxos = computed(() => {
  const start = (utxoPage.value - 1) * utxoPageSize.value
  const end = start + utxoPageSize.value
  
  // Enhance UTXOs with client-side maturity validation
  return utxos.value.slice(start, end).map(utxo => {
    // Check coinbase maturity (100 blocks required)
    const coinbaseMaturity = utxo.coinbase && utxo.confirmations < 100
    // Check coinstake maturity (20 blocks required for PIVX PoS)
    const coinstakeMaturity = utxo.coinstake && utxo.confirmations < 20
    
    // Override spendable status if immature
    const actuallySpendable = utxo.spendable && !coinbaseMaturity && !coinstakeMaturity
    
    // Calculate maturity progress for display
    let maturityInfo = null
    if (coinbaseMaturity) {
      maturityInfo = {
        type: 'coinbase',
        current: utxo.confirmations,
        required: 100,
        remaining: 100 - utxo.confirmations
      }
    } else if (coinstakeMaturity) {
      maturityInfo = {
        type: 'coinstake',
        current: utxo.confirmations,
        required: 20,
        remaining: 20 - utxo.confirmations
      }
    }
    
    return {
      ...utxo,
      actuallySpendable,
      maturityInfo
    }
  })
})

// Methods
const fetchAddressData = async () => {
  loading.value = true
  error.value = null
  
  try {
    // Fetch address data
    const data = await addressService.getAddress(address.value)
    addressData.value = data
    
    // Fetch transaction details
    if (data.txids && data.txids.length > 0) {
      const txDetails = await transactionService.getTransactions(data.txids)
      // Filter out orphaned (height -1) and unresolved (height -2) transactions
      transactions.value = txDetails
        .filter(tx => {
          // Exclude transactions with invalid heights
          const height = tx.blockHeight || tx.height
          return height !== -1 && height !== -2
        })
        .map(tx => ({
          ...tx,
          type: detectTransactionType(tx)
        }))
    }
  } catch (err) {
    console.error('Failed to fetch address data:', err)
    error.value = err.message || 'Failed to load address data'
  } finally {
    loading.value = false
  }
}

const fetchUTXOs = async () => {
  if (utxos.value.length > 0) return // Already loaded
  
  loadingUtxos.value = true
  try {
    const data = await addressService.getUTXOs(address.value)
    // Filter out orphaned (height -1) and unresolved (height -2) UTXOs
    utxos.value = (data || []).filter(utxo => {
      const height = utxo.height || 0
      return height !== -1 && height !== -2
    })
  } catch (err) {
    console.error('Failed to fetch UTXOs:', err)
  } finally {
    loadingUtxos.value = false
  }
}

const generateQR = async (canvas) => {
  if (!canvas) return
  
  try {
    await QRCode.toCanvas(canvas, address.value, {
      width: 256,
      margin: 2,
      color: {
        dark: '#662D91',
        light: '#FFFFFF'
      }
    })
  } catch (err) {
    console.error('Failed to generate QR code:', err)
  }
}

const downloadQR = () => {
  const canvas = qrCanvas.value || qrModalCanvas.value
  if (!canvas) return
  
  const link = document.createElement('a')
  link.download = `pivx-address-${address.value.substring(0, 8)}.png`
  link.href = canvas.toDataURL()
  link.click()
}

const copyAddress = async () => {
  try {
    await navigator.clipboard.writeText(address.value)
    // Could add toast notification here
  } catch (err) {
    console.error('Failed to copy address:', err)
  }
}

const navigateToTransaction = (tx) => {
  router.push(`/tx/${tx.txid}`)
}

// Watch for tab changes
watch(activeTab, async (newTab) => {
  if (newTab === 'utxos' && utxos.value.length === 0) {
    await fetchUTXOs()
  }
  
  if (newTab === 'qr') {
    await nextTick()
    generateQR(qrCanvas.value)
  }
})

// Watch for QR modal
watch(showQR, async (show) => {
  if (show) {
    await nextTick()
    generateQR(qrModalCanvas.value)
  }
})

// Watch route changes
watch(() => route.params.address, (newAddress) => {
  if (newAddress) {
    address.value = newAddress
    fetchAddressData()
  }
})

// Watch for reorg detection and refetch address data
watch(() => chainStore.reorgDetected, (detected) => {
  if (detected && address.value) {
    console.log('🔄 Reorg detected - refetching address data')
    fetchAddressData()
  }
})

onMounted(() => {
  fetchAddressData()
})
</script>

<style scoped>
.address-detail {
  padding: var(--space-6) 0;
}

.breadcrumb {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin-bottom: var(--space-4);
  font-size: var(--text-sm);
  color: var(--text-secondary);
}

.breadcrumb a {
  color: var(--text-accent);
  text-decoration: none;
}

.breadcrumb a:hover {
  text-decoration: underline;
}

.separator {
  color: var(--text-tertiary);
}

.current {
  color: var(--text-primary);
}

.address-header {
  margin-bottom: var(--space-6);
}

.header-content h1 {
  margin-bottom: var(--space-3);
}

.address-value {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex-wrap: wrap;
}

.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
  gap: var(--space-4);
  margin-bottom: var(--space-6);
}

.address-tabs {
  margin-bottom: var(--space-6);
}

.tab-content {
  animation: fadeIn 0.3s ease;
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
}

.section-header h2 {
  margin: 0;
}

.filters {
  display: flex;
  gap: var(--space-3);
}

.filter-select {
  padding: var(--space-2) var(--space-4);
  background: var(--bg-tertiary);
  border: 2px solid var(--border-secondary);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: var(--text-sm);
  cursor: pointer;
}

.filter-select:focus {
  outline: none;
  border-color: var(--border-accent);
}

.transactions-list {
  display: grid;
  gap: var(--space-3);
  margin-bottom: var(--space-6);
}

.utxos-table {
  overflow-x: auto;
  margin-bottom: var(--space-6);
}

.utxos-table table {
  width: 100%;
  border-collapse: collapse;
  background: var(--bg-secondary);
  border-radius: var(--radius-md);
}

.utxos-table thead {
  background: var(--bg-tertiary);
}

.utxos-table th {
  padding: var(--space-3) var(--space-4);
  text-align: left;
  font-size: var(--text-sm);
  font-weight: var(--weight-bold);
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.utxos-table td {
  padding: var(--space-3) var(--space-4);
  border-top: 1px solid var(--border-subtle);
}

.utxos-table tr:hover {
  background: var(--bg-tertiary);
}

.amount {
  font-family: var(--font-mono);
  font-weight: var(--weight-bold);
  color: var(--text-accent);
}

.height-link {
  color: var(--text-accent);
  text-decoration: none;
  font-family: var(--font-mono);
}

.height-link:hover {
  text-decoration: underline;
}

.ml-2 {
  margin-left: var(--space-2);
}

.status-badges {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-wrap: wrap;
}

.qr-card {
  max-width: 500px;
  margin: 0 auto;
}

.qr-content {
  text-align: center;
  padding: var(--space-6);
}

.qr-content h2 {
  margin-bottom: var(--space-6);
}

.qr-code-container {
  display: flex;
  justify-content: center;
  margin-bottom: var(--space-4);
  padding: var(--space-4);
  background: white;
  border-radius: var(--radius-md);
}

.qr-canvas {
  display: block;
}

.qr-address {
  font-family: var(--font-mono);
  font-size: var(--text-sm);
  color: var(--text-secondary);
  word-break: break-all;
  margin-bottom: var(--space-4);
  padding: var(--space-3);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
}

.qr-actions {
  display: flex;
  gap: var(--space-3);
  justify-content: center;
}

.qr-modal-content {
  text-align: center;
  padding: var(--space-4);
}

.qr-modal-content .qr-canvas {
  margin: 0 auto var(--space-4);
}

.download-btn {
  margin-top: var(--space-4);
}

.loading-container,
.error-container {
  display: grid;
  gap: var(--space-4);
}

.error-content {
  text-align: center;
  padding: var(--space-8);
}

.error-icon {
  font-size: 4rem;
  margin-bottom: var(--space-4);
}

.error-content h2 {
  margin-bottom: var(--space-3);
  color: var(--danger);
}

.error-content p {
  color: var(--text-secondary);
  margin-bottom: var(--space-6);
}

.empty-state {
  padding: var(--space-8);
}

@media (max-width: 768px) {
  .stats-grid {
    grid-template-columns: 1fr;
  }
  
  .address-value {
    flex-direction: column;
    align-items: flex-start;
  }
  
  .section-header {
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-3);
  }
  
  .utxos-table {
    font-size: var(--text-sm);
  }
  
  .utxos-table th,
  .utxos-table td {
    padding: var(--space-2) var(--space-3);
  }
}
</style>
