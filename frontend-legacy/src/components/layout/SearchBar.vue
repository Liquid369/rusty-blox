<template>
  <div class="search-bar">
    <input
      v-model="searchQuery"
      type="text"
      class="search-input"
      placeholder="Search block, tx, address..."
      @keyup.enter="handleSearch"
      @focus="showSuggestions = true"
      @blur="hideSuggestions"
    >
    <button 
      class="search-button" 
      @click="handleSearch"
      :disabled="!searchQuery.trim()"
    >
      <Icon name="search" :size="16" />
    </button>

    <!-- Search Suggestions (Recent History) -->
    <div 
      v-if="showSuggestions && settingsStore.searchHistory.length > 0" 
      class="search-suggestions"
    >
      <div class="suggestions-header">
        <span>Recent Searches</span>
        <button @click="clearHistory" class="clear-button">Clear</button>
      </div>
      <div
        v-for="item in settingsStore.searchHistory"
        :key="item"
        class="suggestion-item"
        @mousedown.prevent="selectSuggestion(item)"
      >
        {{ item }}
      </div>
    </div>
  </div>
</template>

<script setup>
import Icon from '@/components/common/Icon.vue'
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import { useSettingsStore } from '@/stores/settingsStore'
import { searchService } from '@/services/searchService'

const router = useRouter()
const settingsStore = useSettingsStore()

const searchQuery = ref('')
const showSuggestions = ref(false)
const isSearching = ref(false)

const handleSearch = async () => {
  const query = searchQuery.value.trim()
  if (!query || isSearching.value) return

  isSearching.value = true
  
  try {
    // Add to search history
    settingsStore.addToSearchHistory(query)
    
    // Perform search
    const result = await searchService.search(query)
    
    // Navigate based on result type
    if (result.type === 'Block') {
      router.push(`/block/${result.height}`)
    } else if (result.type === 'Transaction') {
      router.push(`/tx/${result.txid}`)
    } else if (result.type === 'Address') {
      router.push(`/address/${result.address}`)
    } else {
      // Unknown or not found - go to search results page
      router.push({ 
        name: 'SearchResults', 
        query: { q: query }
      })
    }
    
    // Clear input
    searchQuery.value = ''
    showSuggestions.value = false
  } catch (error) {
    console.error('Search error:', error)
    // Navigate to search results page with error
    router.push({ 
      name: 'SearchResults', 
      query: { q: query, error: 'true' }
    })
  } finally {
    isSearching.value = false
  }
}

const selectSuggestion = (item) => {
  searchQuery.value = item
  handleSearch()
}

const clearHistory = () => {
  settingsStore.clearSearchHistory()
  showSuggestions.value = false
}

const hideSuggestions = () => {
  setTimeout(() => {
    showSuggestions.value = false
  }, 200)
}
</script>

<style scoped>
.search-bar {
  position: relative;
  width: 100%;
}

.search-input {
  width: 100%;
  padding: var(--space-3) var(--space-4);
  padding-right: 48px;
  background: rgba(var(--rgb-purple-darkest), 0.55);
  border: 1px solid rgba(var(--rgb-purple-accent), 0.2);
  border-radius: var(--radius-full);
  backdrop-filter: blur(var(--blur-sm));
  -webkit-backdrop-filter: blur(var(--blur-sm));
  color: var(--text-primary);
  font-size: var(--text-base);
  font-family: var(--font-mono);
  transition:
    background-color var(--transition-fast),
    border-color var(--transition-fast),
    box-shadow var(--transition-fast);
}

.search-input:hover {
  border-color: rgba(var(--rgb-purple-accent), 0.4);
}

.search-input:focus {
  outline: none;
  background: rgba(var(--rgb-purple-darkest), 0.8);
  border-color: var(--green-accent);
  box-shadow: var(--glow-green);
}

.search-input::placeholder {
  color: var(--text-tertiary);
  font-family: var(--font-primary);
}

.search-button {
  position: absolute;
  right: 4px;
  top: 50%;
  transform: translateY(-50%);
  width: 40px;
  height: 40px;
  background: transparent;
  border: none;
  border-radius: var(--radius-full);
  font-size: 18px;
  cursor: pointer;
  transition: background-color var(--transition-fast), transform var(--transition-fast);
  display: flex;
  align-items: center;
  justify-content: center;
}

.search-button:hover:not(:disabled) {
  background: rgba(var(--rgb-purple-accent), 0.35);
  transform: translateY(-50%) scale(1.05);
}

.search-button:focus-visible {
  outline: 2px solid var(--focus-ring-color);
  outline-offset: 2px;
}

.search-button:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.search-suggestions {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  right: 0;
  background: rgba(var(--rgb-purple-darkest), 0.92);
  border: 1px solid var(--glass-border);
  border-radius: var(--radius-md);
  backdrop-filter: blur(var(--blur-md));
  -webkit-backdrop-filter: blur(var(--blur-md));
  box-shadow: var(--shadow-lg), var(--glass-highlight);
  z-index: var(--z-dropdown);
  max-height: 300px;
  overflow-y: auto;
}

.suggestions-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--space-3) var(--space-4);
  border-bottom: 1px solid var(--border-subtle);
  font-size: var(--text-xs);
  color: var(--text-tertiary);
  font-weight: var(--weight-bold);
  text-transform: uppercase;
}

.clear-button {
  background: none;
  border: none;
  color: var(--text-accent);
  font-size: var(--text-xs);
  font-weight: var(--weight-bold);
  cursor: pointer;
  padding: 0;
  transition: color var(--transition-fast);
}

.clear-button:hover {
  color: var(--pivx-accent-dark);
}

.suggestion-item {
  padding: var(--space-3) var(--space-4);
  cursor: pointer;
  font-family: var(--font-mono);
  font-size: var(--text-sm);
  color: var(--text-secondary);
  transition: all var(--transition-fast);
  border-bottom: 1px solid var(--border-subtle);
}

.suggestion-item:last-child {
  border-bottom: none;
}

.suggestion-item:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

@media (max-width: 768px) {
  .search-input {
    font-size: var(--text-sm);
    padding: var(--space-2) var(--space-3);
    padding-right: 44px;
  }

  .search-button {
    width: 36px;
    height: 36px;
    font-size: 16px;
  }
}
</style>
