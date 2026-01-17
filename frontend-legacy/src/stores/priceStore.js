import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import api from '@/services/api'

export const usePriceStore = defineStore('price', () => {
  // State
  const prices = ref({
    usd: 0,
    eur: 0,
    btc: 0
  })
  const lastUpdated = ref(null)
  const loading = ref(false)
  const error = ref(null)

  // Computed
  const hasValidPrices = computed(() => {
    return prices.value.usd > 0 || prices.value.eur > 0 || prices.value.btc > 0
  })

  const isStale = computed(() => {
    if (!lastUpdated.value) return true
    const tenMinutes = 10 * 60 * 1000
    return (Date.now() - lastUpdated.value) > tenMinutes
  })

  // Actions
  const fetchPrices = async () => {
    loading.value = true
    error.value = null
    
    try {
      // Call our backend endpoint instead of CoinGecko directly
      const response = await api.get('/api/v2/price')
      const data = response.data
      
      if (data && (data.usd || data.eur || data.btc)) {
        prices.value = {
          usd: data.usd || 0,
          eur: data.eur || 0,
          btc: data.btc || 0
        }
        // Backend returns Unix timestamp, convert to milliseconds
        lastUpdated.value = data.last_updated ? data.last_updated * 1000 : Date.now()
      } else {
        throw new Error('Invalid price data received')
      }
    } catch (err) {
      console.error('Failed to fetch PIVX prices:', err)
      error.value = err.message || 'Failed to fetch prices'
      
      // Keep last known prices on error
      if (!hasValidPrices.value) {
        // Only reset to zero if we have no valid prices
        prices.value = { usd: 0, eur: 0, btc: 0 }
      }
    } finally {
      loading.value = false
    }
  }

  /**
   * Auto-refresh prices every 60 seconds
   * Returns interval ID for cleanup
   */
  const startAutoRefresh = () => {
    fetchPrices() // Initial fetch
    const intervalId = setInterval(fetchPrices, 60000) // 60 seconds
    return intervalId
  }

  /**
   * Convert PIV amount to fiat currency
   * @param {number} pivAmount - Amount in PIV
   * @param {string} currency - Target currency (USD, EUR, BTC, PIV)
   * @returns {string} Formatted fiat amount
   */
  const convertToFiat = (pivAmount, currency = 'USD') => {
    if (!pivAmount || isNaN(pivAmount)) return '0.00'
    
    const curr = currency.toUpperCase()
    
    if (curr === 'PIV') {
      return pivAmount.toFixed(8)
    }
    
    const rate = prices.value[curr.toLowerCase()]
    
    if (!rate || rate === 0) {
      return '0.00'
    }
    
    const fiatAmount = pivAmount * rate
    
    // BTC needs more decimal places
    if (curr === 'BTC') {
      return fiatAmount.toFixed(8)
    }
    
    return fiatAmount.toFixed(2)
  }

  /**
   * Get formatted price string with symbol
   * @param {number} pivAmount - Amount in PIV
   * @param {string} currency - Target currency
   * @returns {string} Formatted price with symbol (e.g., "$12.34")
   */
  const formatPrice = (pivAmount, currency = 'USD') => {
    const amount = convertToFiat(pivAmount, currency)
    const curr = currency.toUpperCase()
    
    const symbols = {
      USD: '$',
      EUR: '€',
      BTC: '₿',
      PIV: 'PIV'
    }
    
    const symbol = symbols[curr] || curr
    
    if (curr === 'PIV' || curr === 'BTC') {
      return `${amount} ${symbol}`
    }
    
    return `${symbol}${amount}`
  }

  return {
    // State
    prices,
    lastUpdated,
    loading,
    error,
    // Computed
    hasValidPrices,
    isStale,
    // Actions
    fetchPrices,
    startAutoRefresh,
    convertToFiat,
    formatPrice
  }
})
