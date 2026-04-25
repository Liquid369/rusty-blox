<template>
  <div class="price-widget" :class="{ loading: priceStore.loading }">
    <!-- Loading State -->
    <div v-if="priceStore.loading && !priceStore.hasValidPrices" class="price-loading">
      <div class="loading-spinner"></div>
    </div>

    <!-- Error State -->
    <div v-else-if="priceStore.error && !priceStore.hasValidPrices" class="price-error" title="Unable to load price">
      <img src="/PIVX-Shield.svg" alt="PIVX" class="price-icon" />
      <span class="price-label">PIVX:</span>
      <span class="price-value">--</span>
    </div>

    <!-- Price Display -->
    <div v-else class="price-content" @click="toggleCurrency" :title="`Click to change currency (${nextCurrency})`">
      <img src="/PIVX-Shield.svg" alt="PIVX" class="price-icon" />
      <span class="price-label">PIVX:</span>
      <span class="price-value">{{ formattedPrice }}</span>
      <span v-if="priceStore.isStale" class="price-stale" title="Price data is stale">⚠️</span>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import { usePriceStore } from '@/stores/priceStore'
import { useSettingsStore } from '@/stores/settingsStore'

const priceStore = usePriceStore()
const settingsStore = useSettingsStore()

const allCurrencies = ['PIV', 'USD', 'EUR', 'BTC']

const formattedPrice = computed(() => {
  const curr = settingsStore.preferredCurrency
  if (curr === 'PIV') return 'PIV 1.00'
  
  const price = priceStore.prices[curr.toLowerCase()]
  if (!price || price === 0) return `${curr} --`
  
  const symbol = getCurrencySymbol(curr)
  
  if (curr === 'BTC') {
    return `${symbol}${price.toFixed(8)}`
  }
  return `${symbol}${price.toFixed(6)}`
})

const getCurrencySymbol = (currency) => {
  const symbols = {
    USD: '$',
    EUR: '€',
    BTC: '₿',
    PIV: 'PIV'
  }
  return symbols[currency] || currency
}

const nextCurrency = computed(() => {
  const currentIndex = allCurrencies.indexOf(settingsStore.preferredCurrency)
  const nextIndex = (currentIndex + 1) % allCurrencies.length
  return allCurrencies[nextIndex]
})

const toggleCurrency = () => {
  settingsStore.setCurrency(nextCurrency.value)
}
</script>

<style scoped>
.price-widget {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 16px;
  background: rgba(255, 255, 255, 0.05);
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 8px;
  cursor: pointer;
  transition: all 0.2s ease;
  white-space: nowrap;
  user-select: none;
}

.price-widget:hover {
  background: rgba(255, 255, 255, 0.1);
  border-color: var(--pivx-accent);
  transform: translateY(-1px);
}

.price-widget.loading {
  opacity: 0.7;
  cursor: default;
}

.price-loading,
.price-error,
.price-content {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 0.9rem;
}

.price-icon {
  width: 18px;
  height: 18px;
  opacity: 0.9;
}

.price-label {
  color: var(--text-secondary);
  font-weight: 500;
}

.price-value {
  color: var(--pivx-accent);
  font-weight: 700;
  font-size: 1rem;
}

.price-stale {
  font-size: 0.85rem;
  opacity: 0.7;
}

.loading-spinner {
  width: 16px;
  height: 16px;
  border: 2px solid rgba(255, 255, 255, 0.2);
  border-top-color: var(--pivx-accent);
  border-radius: 50%;
  animation: spin 1s linear infinite;
}

@keyframes spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

/* Responsive - hide on small screens */
@media (max-width: 768px) {
  .price-widget {
    display: none;
  }
}
</style>
