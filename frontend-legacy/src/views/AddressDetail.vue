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
              <Icon name="qr-code" :size="14" /> QR
            </Button>
          </div>
        </div>
      </div>

      <!-- Invalid Address State -->
      <div v-if="!isValidAddress" class="error-container">
        <Card>
          <div class="error-content">
            <p class="error-icon"><Icon name="alert-triangle" :size="32" /></p>
            <h2>Invalid Address</h2>
            <p>This is not a valid PIVX address. Please check the value and try again.</p>
            <Button @click="$router.push('/')">Back to Dashboard</Button>
          </div>
        </Card>
      </div>

      <!-- Loading State -->
      <div v-else-if="loading && !addressData" class="loading-container">
        <SkeletonLoader variant="card" height="120px" />
        <SkeletonLoader variant="card" height="400px" />
      </div>

      <!-- Error State -->
      <div v-else-if="error" class="error-container">
        <Card>
          <div class="error-content">
            <p class="error-icon"><Icon name="alert-triangle" :size="32" /></p>
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
            :value="formatPIV(displayBalance, 2)"
            suffix="PIV"
            icon="coins"
            value-class="text-accent"
            :subtitle="balanceInconsistent ? 'Adjusted — value reconciling' : balanceFiat"
          />
          <StatCard
            label="Total Received"
            :value="formatPIV(displayReceived, 2)"
            suffix="PIV"
            icon="arrow-down-left"
            value-class="text-success"
            :subtitle="receivedFiat"
          />
          <StatCard
            label="Total Sent"
            :value="formatPIV(displaySent, 2)"
            suffix="PIV"
            icon="arrow-up-right"
            :value-class="sentInconsistent ? 'text-warning' : ''"
            :subtitle="sentInconsistent ? 'Adjusted — value reconciling' : sentFiat"
          />
          <StatCard
            label="Transactions"
            :value="formatNumber(totalTxCount)"
            icon="chart-bar"
          />
        </div>

        <!-- Tabs -->
        <Tabs v-model="activeTab" :tabs="tabs" class="address-tabs" />

        <!-- Transactions Tab -->
        <div v-if="activeTab === 'transactions'" class="tab-content">
          <div class="section-header">
            <h2>Transaction History</h2>
            <div class="filters">
              <select v-model="txFilter" class="filter-select" aria-label="Filter transactions on this page">
                <option value="all">All on Page</option>
                <option value="received">Received Only</option>
                <option value="sent">Sent Only</option>
              </select>
            </div>
          </div>

          <!-- Per-page loading state -->
          <div v-if="loadingTxs" class="transactions-list">
            <SkeletonLoader variant="card" height="92px" v-for="i in 5" :key="i" />
          </div>

          <div v-else-if="filteredTransactions.length > 0" class="transactions-list">
            <TransactionRow
              v-for="tx in filteredTransactions"
              :key="tx.txid"
              :transaction="tx"
              :viewed-addresses="address"
              @click="navigateToTransaction(tx)"
            />
          </div>

          <div v-else class="empty-state">
            <EmptyState
              icon="inbox"
              :title="txFilter === 'all' ? 'No Transactions' : 'No Matching Transactions'"
              :message="txFilter === 'all'
                ? 'This address has no transaction history.'
                : 'No transactions on this page match the selected filter.'"
            />
          </div>

          <!-- Server-side Pagination -->
          <Pagination
            v-if="totalTxPages > 1"
            :current-page="txPage"
            :page-size="txPageSize"
            :total="totalTxCount"
            @update:page="goToTxPage"
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
              icon="send"
              title="No UTXOs"
              message="This address has no unspent outputs."
            />
          </div>

          <!-- UTXO Pagination -->
          <Pagination
            v-if="totalUtxoPages > 1"
            :current-page="utxoPage"
            :page-size="utxoPageSize"
            :total="utxos.length"
            @update:page="utxoPage = $event"
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
                <Button @click="downloadQR"><Icon name="download" :size="14" /> Download PNG</Button>
                <Button variant="secondary" @click="copyAddress"><Icon name="clipboard" :size="14" /> Copy Address</Button>
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
          <Button @click="downloadQR" class="download-btn"><Icon name="download" :size="14" /> Download</Button>
        </div>
      </Modal>
    </div>
  </AppLayout>
</template>

<script setup>
import Icon from '@/components/common/Icon.vue'
import { ref, computed, watch, onMounted, nextTick } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useChainStore } from '@/stores/chainStore'
import { useCurrency } from '@/composables/useCurrency'
import api from '@/services/api'
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
const chainStore = useChainStore()
const { formatAmount, preferredCurrency, hasValidPrices } = useCurrency()

// Fiat annotation gate (P1-3): only when a non-PIV currency is chosen and
// live prices are available. Falls back to '' (PIV-only) otherwise.
const showFiat = computed(() => preferredCurrency.value !== 'PIV' && hasValidPrices.value)

const address = ref(route.params.address)
const addressData = ref(null)
const transactions = ref([])
const utxos = ref([])
const loading = ref(false)
const loadingTxs = ref(false)
const loadingUtxos = ref(false)
const error = ref(null)

/**
 * Client-side PIVX address validation. Mirrors the backend's base58check guard
 * (src/api/addresses.rs::is_valid_address): we base58-decode and require the
 * decoded byte length to match a real PIVX address class, so a one-char typo
 * that changes the decoded length — or any non-base58 garbage — is rejected
 * here instead of rendering a fake zero account. The backend still verifies the
 * full 4-byte double-SHA256 checksum (the authoritative source of truth); we do
 * not recompute SHA256 in the browser (it would require an async crypto.subtle
 * call inside this synchronous computed), so the backend remains the gate for
 * checksum-only typos that preserve length.
 *
 * Accepted decoded lengths (these are total decoded bytes INCLUDING the 4-byte
 * checksum, i.e. payload 21 or 23 + 4):
 *   - 25 bytes = version(1) + hash160(20) + checksum(4). Covers D (P2PKH v30),
 *     S (cold-staking staker v63), 6/7 (P2SH v13), and single-byte E variants.
 *   - 27 bytes = EXM prefix(3: 0x01,0xb9,0xa2) + hash160(20) + checksum(4).
 *     Covers EXM exchange addresses (OP_EXCHANGEADDR 0xe0, 36 chars).
 */
const BASE58_ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'

/** Decode a base58 string to bytes, or null if it contains a non-base58 char. */
const decodeBase58 = (str) => {
  const bytes = [0]
  for (const ch of str) {
    const val = BASE58_ALPHABET.indexOf(ch)
    if (val === -1) return null
    let carry = val
    for (let j = 0; j < bytes.length; j++) {
      carry += bytes[j] * 58
      bytes[j] = carry & 0xff
      carry >>= 8
    }
    while (carry > 0) {
      bytes.push(carry & 0xff)
      carry >>= 8
    }
  }
  // Each leading '1' in base58 is a leading 0x00 byte.
  for (let k = 0; k < str.length && str[k] === '1'; k++) bytes.push(0)
  return bytes.reverse()
}

const isValidAddress = computed(() => {
  const a = address.value
  if (typeof a !== 'string') return false
  const t = a.trim()
  if (t.length < 26 || t.length > 64) return false
  if (!/^[DSE67]/.test(t)) return false
  const decoded = decodeBase58(t)
  if (!decoded) return false
  // 25 = standard (1-byte version + 20-byte hash + 4-byte checksum);
  // 27 = EXM (3-byte prefix + 20-byte hash + 4-byte checksum).
  return decoded.length === 25 || decoded.length === 27
})

// Tabs
const activeTab = ref('transactions')
const tabs = [
  { value: 'transactions', label: 'Transactions' },
  { value: 'utxos', label: 'UTXOs' },
  { value: 'qr', label: 'QR Code' }
]

// Transaction filtering and server-side pagination
const txFilter = ref('all')
const txPage = ref(1)
const txPageSize = ref(25)
// Total tx count + page count come from the backend, not from txids.length
const totalTxCount = computed(() => addressData.value?.txs ?? 0)
const totalTxPages = computed(() => addressData.value?.totalPages ?? 1)

// UTXO pagination
const utxoPage = ref(1)
const utxoPageSize = ref(25)

// QR Code
const showQR = ref(false)
const qrCanvas = ref(null)
const qrModalCanvas = ref(null)

// Guarded amount display (P1-7): a transient backend inconsistency must never
// render a negative or nonsensical figure (e.g. "-34.2M PIV"). We clamp to a
// sane range and surface a subtle "reconciling" hint instead.
const toAmount = (v) => {
  const n = typeof v === 'string' ? parseFloat(v) : Number(v)
  return Number.isFinite(n) ? n : 0
}
const rawBalance = computed(() => toAmount(addressData.value?.balance))
const rawReceived = computed(() => toAmount(addressData.value?.totalReceived))
const rawSent = computed(() => toAmount(addressData.value?.totalSent))

const displayReceived = computed(() => Math.max(0, rawReceived.value))
const displaySent = computed(() => Math.max(0, rawSent.value))
const displayBalance = computed(() => Math.max(0, rawBalance.value))

// Inconsistency flags drive a subtle indicator without hiding the data entirely.
const sentInconsistent = computed(() =>
  rawSent.value < 0 || rawSent.value > rawReceived.value
)
const balanceInconsistent = computed(() =>
  rawBalance.value < 0 || rawBalance.value > rawReceived.value
)

// Muted fiat annotations for the balance StatCards. Balances are satoshi floats,
// so divide by 1e8 to the PIV value before converting — never double-scale.
const fiatSubtitle = (satsAmount) => {
  if (!showFiat.value) return ''
  return `≈ ${formatAmount(satsAmount / 100000000, { showPIV: false })}`
}
const balanceFiat = computed(() => fiatSubtitle(displayBalance.value))
const receivedFiat = computed(() => fiatSubtitle(displayReceived.value))
const sentFiat = computed(() => fiatSubtitle(displaySent.value))

// Filter operates over the current page's resolved transactions (server-paged).
const filteredTransactions = computed(() => {
  if (txFilter.value === 'all') return transactions.value

  return transactions.value.filter(tx => {
    // Determine if this address received or sent based on inputs/outputs
    const isReceived = tx.vout?.some(output =>
      output.scriptPubKey?.addresses?.includes(address.value) ||
      output.addresses?.includes(address.value)
    )
    const isSent = tx.vin?.some(input =>
      input.addresses?.includes(address.value)
    )

    if (txFilter.value === 'received') return isReceived && !isSent
    if (txFilter.value === 'sent') return isSent
    return true
  })
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

// Monotonic token so a stale in-flight fetch (rapid page clicks / route change)
// can't clobber the latest result.
let fetchToken = 0

/**
 * Fetch a single server-side page of the address.
 *
 * The backend (/api/v2/address/{addr}?page=N&pageSize=M) returns this page's
 * txids plus totalPages and txs (the real total tx count). We resolve details
 * for only this page's ~25 txids with bounded concurrency — never the whole
 * history at once — so there is no unbounded fan-out.
 *
 * @param {number} page
 * @param {boolean} initial - true on first load / address change (full skeleton)
 */
const fetchAddressData = async (page = 1, initial = true) => {
  if (!isValidAddress.value) {
    addressData.value = null
    transactions.value = []
    error.value = null
    loading.value = false
    return
  }

  const token = ++fetchToken
  error.value = null
  if (initial) {
    loading.value = true
    addressData.value = null
    transactions.value = []
    txPage.value = 1
    page = 1
  }
  loadingTxs.value = true

  try {
    // Paged address request (page + pageSize). addressService.getAddress does
    // not forward pageSize, so we call the shared api instance directly.
    const { data } = await api.get(`/api/v2/address/${address.value}`, {
      params: { page, pageSize: txPageSize.value, _cb: Date.now() }
    })
    if (token !== fetchToken) return // superseded

    addressData.value = data
    txPage.value = data.page || page

    const pageTxids = Array.isArray(data.txids) ? data.txids : []
    if (pageTxids.length > 0) {
      // Resolve only this page's txids, with a bounded concurrency window.
      const txDetails = await transactionService.getTransactions(pageTxids)
      if (token !== fetchToken) return // superseded

      transactions.value = txDetails
        .filter(tx => {
          // Exclude orphaned (-1) and unresolved (-2) transactions
          const height = tx.blockHeight ?? tx.height
          return height !== -1 && height !== -2
        })
        .map(tx => ({ ...tx, type: detectTransactionType(tx) }))
    } else {
      transactions.value = []
    }
  } catch (err) {
    if (token !== fetchToken) return
    error.value = err.message || 'Failed to load address data'
  } finally {
    if (token === fetchToken) {
      loading.value = false
      loadingTxs.value = false
    }
  }
}

// Drive the Pagination component: fetch the requested server page.
const goToTxPage = (page) => {
  if (page === txPage.value) return
  fetchAddressData(page, false)
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
    // Reset per-address view state so stale data never carries over
    txFilter.value = 'all'
    utxos.value = []
    utxoPage.value = 1
    activeTab.value = 'transactions'
    fetchAddressData(1, true)
  }
})

// Watch for reorg detection and refetch the current page
watch(() => chainStore.reorgDetected, (detected) => {
  if (detected && address.value && isValidAddress.value) {
    fetchAddressData(txPage.value, false)
  }
})

onMounted(() => {
  fetchAddressData(1, true)
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
  background: rgba(var(--rgb-purple-darkest), 0.55);
  border: 1px solid var(--border-secondary);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: var(--text-sm);
  cursor: pointer;
  transition: border-color var(--transition-fast), box-shadow var(--transition-fast);
}

.filter-select:hover {
  border-color: rgba(var(--rgb-purple-accent), 0.45);
}

.filter-select:focus {
  outline: none;
  border-color: var(--border-accent);
  box-shadow: var(--focus-ring-glow);
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
  background: var(--glass-bg-subtle);
  border: 1px solid var(--border-secondary);
  border-radius: var(--radius-md);
}

.utxos-table th {
  padding: var(--space-3) var(--space-4);
  text-align: left;
  font-size: var(--text-xs);
  font-weight: var(--weight-semibold);
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
  background: rgba(var(--rgb-purple-darkest), 0.92);
  border-bottom: 1px solid var(--border-primary);
  position: sticky;
  top: 0;
  z-index: 1;
}

.utxos-table th:nth-child(3),
.utxos-table th:nth-child(4),
.utxos-table th:nth-child(5),
.utxos-table td:nth-child(3),
.utxos-table td:nth-child(4),
.utxos-table td:nth-child(5) {
  text-align: right;
}

.utxos-table td {
  padding: var(--space-3) var(--space-4);
  border-top: 1px solid var(--border-subtle);
  font-variant-numeric: tabular-nums;
}

.utxos-table tbody tr {
  transition: background-color var(--transition-fast);
}

.utxos-table tbody tr:hover {
  background: var(--bg-hover);
}

.amount {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
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
