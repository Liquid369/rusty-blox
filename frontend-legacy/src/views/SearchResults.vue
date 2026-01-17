<template>
  <AppLayout>
    <div class="search-results">
      <div class="breadcrumb">
        <RouterLink to="/">Home</RouterLink>
        <span class="separator">/</span>
        <span class="current">Search</span>
      </div>

      <div class="search-header">
        <h1>Search Results</h1>
        <p v-if="searchQuery" class="search-query">
          Results for: <span class="query-text">{{ searchQuery }}</span>
        </p>
      </div>

      <!-- Loading State -->
      <div v-if="searching" class="loading-container">
        <LoadingSpinner size="lg" />
        <p>Searching blockchain...</p>
      </div>

      <!-- Error State -->
      <div v-else-if="error" class="error-container">
        <Card>
          <div class="error-content">
            <p class="error-icon">⚠️</p>
            <h2>Search Error</h2>
            <p>{{ error }}</p>
            <Button @click="performSearch">Try Again</Button>
          </div>
        </Card>
      </div>

      <!-- Results -->
      <div v-else-if="results" class="results-container">
        <!-- No Results -->
        <div v-if="totalResults === 0" class="no-results">
          <EmptyState
            icon="🔍"
            title="No Results Found"
            :message="`No blocks, transactions, or addresses match '${searchQuery}'`"
          />
        </div>

        <!-- Results Found -->
        <div v-else class="results-content">
          <div class="results-summary">
            <Badge variant="info">{{ totalResults }} Result{{ totalResults !== 1 ? 's' : '' }}</Badge>
          </div>

          <!-- Blocks -->
          <section v-if="results.blocks && results.blocks.length > 0" class="result-section">
            <h2>Blocks ({{ results.blocks.length }})</h2>
            <div class="results-grid">
              <BlockCard
                v-for="block in results.blocks"
                :key="block.height"
                :block="block"
                @click="navigateToBlock(block)"
              />
            </div>
          </section>

          <!-- Transactions -->
          <section v-if="results.transactions && results.transactions.length > 0" class="result-section">
            <h2>Transactions ({{ results.transactions.length }})</h2>
            <div class="transactions-list">
              <TransactionRow
                v-for="tx in results.transactions"
                :key="tx.txid"
                :transaction="tx"
                @click="navigateToTransaction(tx)"
              />
            </div>
          </section>

          <!-- Addresses -->
          <section v-if="results.addresses && results.addresses.length > 0" class="result-section">
            <h2>Addresses ({{ results.addresses.length }})</h2>
            <div class="addresses-list">
              <Card
                v-for="address in results.addresses"
                :key="address.address"
                class="address-card"
                hover
                @click="navigateToAddress(address)"
              >
                <div class="address-info">
                  <HashDisplay :hash="address.address" :short="false" :copyable="true" />
                  <div class="address-stats">
                    <span class="stat">Balance: {{ formatPIV(address.balance) }} PIV</span>
                    <span class="stat">Txs: {{ formatNumber(address.txCount || 0) }}</span>
                  </div>
                </div>
              </Card>
            </div>
          </section>
        </div>
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, computed, onMounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { searchService } from '@/services/searchService'
import { formatPIV, formatNumber } from '@/utils/formatters'
import { detectTransactionType } from '@/utils/transactionHelpers'
import AppLayout from '@/components/layout/AppLayout.vue'
import Card from '@/components/common/Card.vue'
import Badge from '@/components/common/Badge.vue'
import Button from '@/components/common/Button.vue'
import LoadingSpinner from '@/components/common/LoadingSpinner.vue'
import EmptyState from '@/components/common/EmptyState.vue'
import HashDisplay from '@/components/common/HashDisplay.vue'
import BlockCard from '@/components/common/BlockCard.vue'
import TransactionRow from '@/components/common/TransactionRow.vue'

const route = useRoute()
const router = useRouter()

const searchQuery = ref(route.query.q || '')
const searching = ref(false)
const results = ref(null)
const error = ref(null)

const totalResults = computed(() => {
  if (!results.value) return 0
  const blocks = results.value.blocks?.length || 0
  const transactions = results.value.transactions?.length || 0
  const addresses = results.value.addresses?.length || 0
  return blocks + transactions + addresses
})

const performSearch = async () => {
  if (!searchQuery.value) {
    error.value = 'Please enter a search query'
    return
  }

  searching.value = true
  error.value = null
  results.value = null

  try {
    const data = await searchService.search(searchQuery.value)
    
    // Process transactions to add type
    if (data.transactions) {
      data.transactions = data.transactions.map(tx => ({
        ...tx,
        type: detectTransactionType(tx)
      }))
    }
    
    results.value = data
  } catch (err) {
    console.error('Search error:', err)
    error.value = err.message || 'Failed to perform search'
  } finally {
    searching.value = false
  }
}

const navigateToBlock = (block) => {
  router.push(`/block/${block.height}`)
}

const navigateToTransaction = (tx) => {
  router.push(`/tx/${tx.txid}`)
}

const navigateToAddress = (address) => {
  router.push(`/address/${address.address}`)
}

// Watch for route query changes
watch(() => route.query.q, (newQuery) => {
  searchQuery.value = newQuery || ''
  if (searchQuery.value) {
    performSearch()
  }
})

onMounted(() => {
  if (searchQuery.value) {
    performSearch()
  }
})
</script>

<style scoped>
.search-results {
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

.search-header {
  margin-bottom: var(--space-6);
}

.search-header h1 {
  margin-bottom: var(--space-2);
}

.search-query {
  color: var(--text-secondary);
  font-size: var(--text-lg);
}

.query-text {
  color: var(--text-accent);
  font-weight: var(--weight-bold);
  font-family: var(--font-mono);
}

.loading-container {
  text-align: center;
  padding: var(--space-12);
}

.loading-container p {
  margin-top: var(--space-4);
  color: var(--text-secondary);
}

.error-container {
  padding: var(--space-6) 0;
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

.no-results {
  padding: var(--space-8);
}

.results-container {
  animation: fadeIn 0.3s ease;
}

@keyframes fadeIn {
  from { opacity: 0; }
  to { opacity: 1; }
}

.results-summary {
  margin-bottom: var(--space-6);
}

.result-section {
  margin-bottom: var(--space-8);
}

.result-section h2 {
  margin-bottom: var(--space-4);
  color: var(--text-primary);
}

.results-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  gap: var(--space-4);
}

.transactions-list,
.addresses-list {
  display: grid;
  gap: var(--space-3);
}

.address-card {
  cursor: pointer;
}

.address-info {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.address-stats {
  display: flex;
  gap: var(--space-4);
  font-size: var(--text-sm);
}

.stat {
  color: var(--text-secondary);
}

.stat:first-child {
  color: var(--text-accent);
  font-weight: var(--weight-bold);
}

@media (max-width: 768px) {
  .results-grid {
    grid-template-columns: 1fr;
  }
  
  .address-stats {
    flex-direction: column;
    gap: var(--space-2);
  }
}
</style>
