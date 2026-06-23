<template>
  <AppLayout>
    <div class="address-detail-page">
      <h1>Address</h1>
      <div class="address-line">
        <span class="mono address-text">{{ address }}</span>
        <span v-if="isColdStakingAddress" class="badge badge-info">Cold Staking</span>
      </div>

      <div v-if="loading && !addressData" class="skeleton-list mt-6">
        <div class="skeleton" style="height: 120px;"></div>
        <div class="skeleton" style="height: 400px;"></div>
      </div>

      <div v-else-if="error" class="error-message">
        <h2>Address Not Found</h2>
        <p class="text-secondary">{{ error }}</p>
        <UiButton @click="$router.push('/')">Go to Dashboard</UiButton>
      </div>

      <div v-else-if="addressData">
        <div class="stats-grid mt-6">
          <StatCard
            label="Balance"
            :value="formatSats(addressData.balance)"
            subtitle="PIV"
          />
          <StatCard
            label="Total Received"
            :value="formatSats(addressData.totalReceived)"
            subtitle="PIV"
          />
          <StatCard
            label="Total Sent"
            :value="formatSats(addressData.totalSent)"
            subtitle="PIV"
          />
          <StatCard
            label="Transactions"
            :value="addressData.txs"
            format="number"
          />
        </div>

        <div class="section-header mt-8">
          <h2>Transactions</h2>
          <span v-if="totalPages > 1" class="text-tertiary page-label">
            Page {{ page }} of {{ totalPages.toLocaleString() }}
          </span>
        </div>

        <div v-if="loadingTxs" class="skeleton-list mt-6">
          <div v-for="i in 5" :key="i" class="skeleton" style="height: 90px;"></div>
        </div>

        <div v-else-if="transactions.length" class="tx-list mt-6">
          <UiCard
            v-for="tx in transactions"
            :key="tx.txid"
            hover
            clickable
            @click="goToTx(tx.txid)"
          >
            <div class="tx-row">
              <div class="tx-main">
                <span class="mono tx-id">{{ truncateHash(tx.txid) }}</span>
                <span class="tx-time">{{ tx.blockTime ? formatDate(tx.blockTime) : 'Pending' }}</span>
              </div>
              <div class="tx-side">
                <span class="mono tx-amount" :class="netAmount(tx) >= 0 ? 'amount-in' : 'amount-out'">
                  {{ netAmount(tx) >= 0 ? '+' : '' }}{{ formatPivFloat(netAmount(tx)) }} PIV
                </span>
                <span v-if="tx.blockHeight > 0" class="tx-block">
                  Block #{{ tx.blockHeight.toLocaleString() }}
                </span>
                <span v-else class="badge badge-warning">Unconfirmed</span>
              </div>
            </div>
          </UiCard>
        </div>

        <div v-else class="empty-state mt-6">
          <UiCard>
            <p class="text-tertiary empty-text">This address has no transaction history.</p>
          </UiCard>
        </div>

        <div v-if="totalPages > 1" class="pagination mt-8">
          <UiButton :disabled="page <= 1 || loadingTxs" @click="goToPage(page - 1)">
            ← Previous
          </UiButton>
          <span class="page-info">Page {{ page }} of {{ totalPages.toLocaleString() }}</span>
          <UiButton :disabled="page >= totalPages || loadingTxs" @click="goToPage(page + 1)">
            Next →
          </UiButton>
        </div>
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { addressService, transactionService } from '@/services'
import AppLayout from '@/components/layout/AppLayout.vue'
import StatCard from '@/components/common/StatCard.vue'
import UiCard from '@/components/common/UiCard.vue'
import UiButton from '@/components/common/UiButton.vue'

const PAGE_SIZE = 25

const route = useRoute()
const router = useRouter()

const address = ref(route.params.address)
const addressData = ref(null)
const transactions = ref([])
const loading = ref(true)
const loadingTxs = ref(false)
const error = ref('')
const page = ref(1)
const totalPages = ref(1)

// PIVX cold-staking addresses start with "S" and are valid
const isColdStakingAddress = computed(() => address.value?.startsWith('S'))

// Balances arrive as string satoshis - divide by 1e8 only for display
const formatSats = (value) => {
  if (value === null || value === undefined || value === '') return '0.00'
  const piv = Number(value) / 100000000
  return piv.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 8 })
}

const formatPivFloat = (piv) => {
  return piv.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 8 })
}

// Net effect of a tx on this address (PIV float, display only)
const netAmount = (tx) => {
  let sats = 0
  for (const output of tx.vout || []) {
    if (output.addresses?.includes(address.value)) sats += Number(output.value)
  }
  for (const input of tx.vin || []) {
    if (input.addresses?.includes(address.value)) sats -= Number(input.value)
  }
  return sats / 100000000
}

const formatDate = (timestamp) => {
  return new Date(timestamp * 1000).toLocaleString()
}

const truncateHash = (hash) => {
  if (!hash) return ''
  return `${hash.slice(0, 12)}...${hash.slice(-12)}`
}

const goToTx = (txid) => {
  router.push(`/tx/${txid}`)
}

const goToPage = (newPage) => {
  if (newPage < 1 || newPage > totalPages.value) return
  page.value = newPage
  loadPage()
}

const loadPage = async () => {
  loadingTxs.value = true
  error.value = ''

  try {
    const data = await addressService.getAddress(address.value, page.value, PAGE_SIZE)
    addressData.value = data
    page.value = data.page || 1
    totalPages.value = data.totalPages || 1

    if (data.txids?.length) {
      transactions.value = await transactionService.getTransactions(data.txids)
    } else {
      transactions.value = []
    }
  } catch (err) {
    error.value = err.response?.data?.error?.message || 'Failed to load address data.'
    addressData.value = null
    transactions.value = []
  } finally {
    loading.value = false
    loadingTxs.value = false
  }
}

watch(() => route.params.address, (newAddress) => {
  if (newAddress) {
    address.value = newAddress
    addressData.value = null
    page.value = 1
    loading.value = true
    loadPage()
  }
}, { immediate: true })
</script>

<style scoped>
.address-detail-page {
  animation: fadeIn 0.3s ease;
}

.address-line {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex-wrap: wrap;
  margin-top: var(--space-3);
}

.address-text {
  font-size: var(--text-lg);
  color: var(--text-secondary);
  word-break: break-all;
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

.page-label {
  font-size: var(--text-sm);
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
  gap: var(--space-2);
}

.tx-amount {
  font-size: var(--text-base);
  font-weight: var(--weight-bold);
}

.amount-in {
  color: var(--success);
}

.amount-out {
  color: var(--danger);
}

.tx-block {
  font-size: var(--text-sm);
  color: var(--text-tertiary);
}

.empty-text {
  text-align: center;
  padding: var(--space-6);
  margin: 0;
}

.pagination {
  display: flex;
  justify-content: center;
  align-items: center;
  gap: var(--space-4);
}

.page-info {
  color: var(--text-secondary);
  font-size: var(--text-sm);
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
