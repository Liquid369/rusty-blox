<template>
  <AppLayout>
    <div class="xpub-detail-page">
      <h1>Extended Public Key</h1>
      <p class="mono xpub-text">{{ xpub }}</p>

      <div v-if="loading" class="skeleton-list mt-6">
        <div class="skeleton" style="height: 120px;"></div>
        <div class="skeleton" style="height: 400px;"></div>
      </div>

      <div v-else-if="error" class="error-message">
        <h2>Invalid XPub</h2>
        <p class="text-secondary">{{ error }}</p>
        <UiButton @click="$router.push('/')">Go to Dashboard</UiButton>
      </div>

      <div v-else-if="xpubData">
        <div class="stats-grid mt-6">
          <StatCard
            label="Balance"
            :value="formatSats(xpubData.balance)"
            subtitle="PIV"
          />
          <StatCard
            label="Total Received"
            :value="formatSats(xpubData.totalReceived)"
            subtitle="PIV"
          />
          <StatCard
            label="Total Sent"
            :value="formatSats(xpubData.totalSent)"
            subtitle="PIV"
          />
          <StatCard
            label="Total Transfers"
            :value="xpubData.txs"
            format="number"
          />
          <StatCard
            label="Used Addresses"
            :value="xpubData.usedTokens || tokens.length"
            format="number"
          />
        </div>

        <div class="section-header mt-8">
          <h2>Derived Addresses</h2>
          <span v-if="totalPages > 1" class="text-tertiary page-label">
            Page {{ page }} of {{ totalPages }}
          </span>
        </div>

        <UiCard v-if="tokens.length" class="mt-6">
          <div class="table-wrap">
            <table class="tokens-table">
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
                <tr v-for="token in paginatedTokens" :key="token.name">
                  <td class="mono path-cell">{{ token.path }}</td>
                  <td>
                    <router-link :to="`/address/${token.name}`" class="mono address-link">
                      {{ truncateHash(token.name) }}
                    </router-link>
                  </td>
                  <td class="mono amount-cell">{{ formatSats(token.balance) }}</td>
                  <td class="mono">{{ formatSats(token.totalReceived) }}</td>
                  <td class="mono">{{ formatSats(token.totalSent) }}</td>
                  <td>{{ token.transfers }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </UiCard>

        <UiCard v-else class="mt-6">
          <p class="text-tertiary empty-text">No used addresses found for this xpub.</p>
        </UiCard>

        <div v-if="totalPages > 1" class="pagination mt-8">
          <UiButton :disabled="page <= 1" @click="page--">← Previous</UiButton>
          <span class="page-info">Page {{ page }} of {{ totalPages }}</span>
          <UiButton :disabled="page >= totalPages" @click="page++">Next →</UiButton>
        </div>
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { useRoute } from 'vue-router'
import { xpubService } from '@/services'
import AppLayout from '@/components/layout/AppLayout.vue'
import StatCard from '@/components/common/StatCard.vue'
import UiCard from '@/components/common/UiCard.vue'
import UiButton from '@/components/common/UiButton.vue'

const PAGE_SIZE = 20

const route = useRoute()

const xpub = ref(route.params.xpub)
const xpubData = ref(null)
const loading = ref(true)
const error = ref('')
const page = ref(1)

const tokens = computed(() => xpubData.value?.tokens || [])

const totalPages = computed(() => Math.max(1, Math.ceil(tokens.value.length / PAGE_SIZE)))

const paginatedTokens = computed(() => {
  const start = (page.value - 1) * PAGE_SIZE
  return tokens.value.slice(start, start + PAGE_SIZE)
})

// Balances arrive as string satoshis - divide by 1e8 only for display
const formatSats = (value) => {
  if (value === null || value === undefined || value === '') return '0.00'
  const piv = Number(value) / 100000000
  return piv.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 8 })
}

const truncateHash = (hash) => {
  if (!hash) return ''
  if (hash.length <= 24) return hash
  return `${hash.slice(0, 12)}...${hash.slice(-12)}`
}

const loadXPub = async () => {
  loading.value = true
  error.value = ''
  xpubData.value = null
  page.value = 1

  try {
    xpubData.value = await xpubService.getXPub(xpub.value, {
      details: 'tokens',
      tokens: 'used',
      tokensPageSize: 100
    })
  } catch (err) {
    error.value = err.response?.data?.error?.message || 'Failed to load xpub data. Please check the key and try again.'
  } finally {
    loading.value = false
  }
}

watch(() => route.params.xpub, (newXpub) => {
  if (newXpub) {
    xpub.value = newXpub
    loadXPub()
  }
}, { immediate: true })
</script>

<style scoped>
.xpub-detail-page {
  animation: fadeIn 0.3s ease;
}

.xpub-text {
  margin-top: var(--space-3);
  font-size: var(--text-sm);
  color: var(--text-secondary);
  word-break: break-all;
  padding: var(--space-3);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
}

.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
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

.table-wrap {
  overflow-x: auto;
}

.tokens-table {
  width: 100%;
  border-collapse: collapse;
}

.tokens-table th {
  padding: var(--space-3) var(--space-4);
  text-align: left;
  font-size: var(--text-xs);
  font-weight: var(--weight-bold);
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.05em;
  border-bottom: 2px solid var(--border-primary);
}

.tokens-table td {
  padding: var(--space-3) var(--space-4);
  border-bottom: 1px solid var(--border-subtle);
  font-size: var(--text-sm);
  color: var(--text-primary);
  white-space: nowrap;
}

.tokens-table tr:last-child td {
  border-bottom: none;
}

.path-cell {
  color: var(--text-tertiary);
}

.amount-cell {
  font-weight: var(--weight-bold);
  color: var(--text-accent);
}

.address-link {
  color: var(--text-accent);
  text-decoration: none;
}

.address-link:hover {
  text-decoration: underline;
}

.empty-text {
  text-align: center;
  padding: var(--space-6);
  margin: 0;
}

.skeleton-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
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
}
</style>
