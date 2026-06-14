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

    <!-- Price Display + discoverable currency selector -->
    <div v-else class="price-content">
      <img src="/PIVX-Shield.svg" alt="PIVX" class="price-icon" />
      <span class="price-label">PIVX:</span>
      <span class="price-value">{{ formattedPrice }}</span>
      <span v-if="priceStore.isStale" class="price-stale" title="Price data is stale"><Icon name="alert-triangle" :size="14" /></span>

      <div class="currency-selector" role="group" aria-label="Display currency">
        <button
          v-for="curr in allCurrencies"
          :key="curr"
          type="button"
          class="currency-option"
          :class="{ active: curr === settingsStore.preferredCurrency }"
          :aria-pressed="curr === settingsStore.preferredCurrency"
          :title="`Show amounts in ${curr}`"
          @click="selectCurrency(curr)"
        >
          {{ curr }}
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import Icon from './Icon.vue'
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

const selectCurrency = (currency) => {
  settingsStore.setCurrency(currency)
}
</script>

<style scoped>
.price-widget {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-4);
  background: rgba(var(--rgb-purple-darkest), 0.45);
  border: 1px solid rgba(var(--rgb-purple-accent), 0.2);
  border-radius: var(--radius-sm);
  backdrop-filter: blur(var(--blur-sm));
  -webkit-backdrop-filter: blur(var(--blur-sm));
  transition:
    background-color var(--transition-fast),
    border-color var(--transition-fast),
    box-shadow var(--transition-fast);
  white-space: nowrap;
}

.price-widget:hover {
  background: rgba(var(--rgb-purple-darkest), 0.7);
  border-color: var(--purple-accent);
  box-shadow: var(--glow-purple);
}

.price-widget.loading {
  opacity: 0.7;
}

.currency-selector {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  margin-left: var(--space-2);
  padding: 2px;
  background: rgba(var(--rgb-purple-darkest), 0.6);
  border: 1px solid rgba(var(--rgb-purple-accent), 0.2);
  border-radius: var(--radius-sm);
}

.currency-option {
  appearance: none;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  font-family: var(--font-primary);
  font-size: var(--text-xs);
  font-weight: var(--weight-semibold);
  letter-spacing: 0.4px;
  padding: 2px var(--space-2);
  border-radius: var(--radius-xs);
  cursor: pointer;
  transition:
    background-color var(--transition-fast),
    color var(--transition-fast);
}

.currency-option:hover {
  color: var(--text-primary);
  background: rgba(var(--rgb-purple-accent), 0.15);
}

.currency-option:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring-glow);
}

.currency-option.active {
  color: var(--bg-primary);
  background: var(--purple-accent);
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
  color: var(--text-primary);
  font-weight: var(--weight-medium);
}

.price-value {
  color: #CD97F7;
  font-weight: var(--weight-bold);
  font-size: var(--text-base);
  font-variant-numeric: tabular-nums;
}

.price-stale {
  font-size: 0.85rem;
  opacity: 0.7;
}

.loading-spinner {
  width: 16px;
  height: 16px;
  border: 2px solid rgba(255, 255, 255, 0.2);
  border-top-color: var(--purple-accent);
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
