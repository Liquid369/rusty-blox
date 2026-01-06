<template>
  <div class="search-bar">
    <input
      v-model="query"
      type="text"
      placeholder="Search blocks, txs, addresses..."
      class="search-input"
      @keyup.enter="handleSearch"
    />
    <button @click="handleSearch" class="search-button" aria-label="Search">
      🔍
    </button>
  </div>
</template>

<script setup>
import { ref } from 'vue'
import { useRouter } from 'vue-router'

const router = useRouter()
const query = ref('')

const handleSearch = () => {
  if (query.value.trim()) {
    router.push({ name: 'SearchResults', query: { q: query.value.trim() } })
  }
}
</script>

<style scoped>
.search-bar {
  display: flex;
  gap: var(--space-2);
  max-width: 400px;
  flex: 1;
}

.search-input {
  flex: 1;
  height: 44px;
  padding: 0 var(--space-4);
  background: var(--bg-tertiary);
  border: 2px solid var(--border-primary);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-family: var(--font-primary);
  font-size: var(--text-base);
  transition: border-color var(--transition-base);
}

.search-input:focus {
  border-color: var(--border-accent);
  box-shadow: var(--shadow-glow);
}

.search-input::placeholder {
  color: var(--text-tertiary);
}

.search-button {
  width: 44px;
  height: 44px;
  background: var(--pivx-purple-primary);
  border: none;
  border-radius: var(--radius-sm);
  font-size: 18px;
  cursor: pointer;
  transition: all var(--transition-base);
}

.search-button:hover {
  background: var(--pivx-purple-light);
  transform: translateY(-2px);
  box-shadow: var(--shadow-md);
}

@media (max-width: 768px) {
  .search-bar {
    max-width: 100%;
  }
}
</style>
