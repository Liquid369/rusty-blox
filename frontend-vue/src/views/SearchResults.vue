<template>
  <AppLayout>
    <div class="search-results-page">
      <h1>Search Results</h1>
      <p v-if="query" class="search-query text-secondary">
        Results for: <span class="mono query-text">{{ query }}</span>
      </p>

      <div v-if="searching" class="loading-container">
        <div class="loading-spinner"></div>
        <p class="text-secondary mt-4">Searching blockchain...</p>
      </div>

      <div v-else-if="!query" class="error-message">
        <h2>No Search Query</h2>
        <p class="text-secondary">Enter a block height, block hash, transaction ID, or address in the search bar.</p>
        <UiButton @click="$router.push('/')">Go to Dashboard</UiButton>
      </div>

      <div v-else-if="error" class="error-message">
        <h2>Search Error</h2>
        <p class="text-secondary">{{ error }}</p>
        <UiButton @click="performSearch">Try Again</UiButton>
      </div>

      <div v-else-if="notFound" class="error-message">
        <h2>No Results Found</h2>
        <p class="text-secondary">
          No blocks, transactions, or addresses match
          <span class="mono query-text">{{ query }}</span>
        </p>
        <UiButton @click="$router.push('/')">Go to Dashboard</UiButton>
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { searchService } from '@/services'
import AppLayout from '@/components/layout/AppLayout.vue'
import UiButton from '@/components/common/UiButton.vue'

const route = useRoute()
const router = useRouter()

const query = ref('')
const searching = ref(false)
const notFound = ref(false)
const error = ref('')

const performSearch = async () => {
  if (!query.value) return

  searching.value = true
  notFound.value = false
  error.value = ''

  try {
    const data = await searchService.search(query.value)

    // Backend returns a single tagged SearchResult - redirect to the right page
    if (data.type === 'Block') {
      await router.replace(`/block/${data.height}`)
      return
    }
    if (data.type === 'Transaction') {
      await router.replace(`/tx/${data.txid}`)
      return
    }
    if (data.type === 'Address') {
      await router.replace(`/address/${data.address}`)
      return
    }
    if (data.type === 'XPub') {
      await router.replace(`/xpub/${data.xpub}`)
      return
    }
    notFound.value = true
  } catch (err) {
    error.value = err.response?.data?.error?.message || 'Failed to perform search.'
  } finally {
    searching.value = false
  }
}

watch(() => route.query.q, (newQuery) => {
  query.value = (newQuery || '').toString().trim()
  if (query.value) {
    performSearch()
  }
}, { immediate: true })
</script>

<style scoped>
.search-results-page {
  animation: fadeIn 0.3s ease;
}

.search-query {
  margin-top: var(--space-3);
  font-size: var(--text-lg);
}

.query-text {
  color: var(--text-accent);
  font-weight: var(--weight-bold);
  word-break: break-all;
}

.loading-container {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: var(--space-16) var(--space-6);
}

.error-message {
  text-align: center;
  padding: var(--space-16) var(--space-6);
}

.error-message p {
  margin-bottom: var(--space-6);
}
</style>
