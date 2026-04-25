import { computed } from 'vue'
import { usePriceStore } from '@/stores/priceStore'
import { useSettingsStore } from '@/stores/settingsStore'

/**
 * Currency formatting composable
 * Provides utilities for displaying PIV amounts in different currencies
 */
export function useCurrency() {
  const priceStore = usePriceStore()
  const settingsStore = useSettingsStore()

  /**
   * Format PIV amount with optional fiat conversion
   * @param {number} pivAmount - Amount in PIV
   * @param {object} options - Formatting options
   * @returns {string} Formatted amount string
   */
  const formatAmount = (pivAmount, options = {}) => {
    const {
      currency = settingsStore.preferredCurrency,
      showSymbol = true,
      showPIV = true,
      decimals = null
    } = options

    if (!pivAmount || isNaN(pivAmount)) {
      return showSymbol ? '0 PIV' : '0'
    }

    // If currency is PIV or we have no price data, just return PIV amount
    if (currency === 'PIV' || !priceStore.hasValidPrices) {
      const formatted = decimals !== null 
        ? pivAmount.toFixed(decimals)
        : pivAmount.toFixed(8)
      return showSymbol ? `${formatted} PIV` : formatted
    }

    // Convert to fiat
    const fiatAmount = priceStore.convertToFiat(pivAmount, currency)
    const symbol = getCurrencySymbol(currency)

    let result
    if (currency === 'BTC') {
      result = showSymbol ? `${fiatAmount} ${symbol}` : fiatAmount
    } else {
      result = showSymbol ? `${symbol}${fiatAmount}` : fiatAmount
    }

    // Optionally show PIV amount in parentheses
    if (showPIV) {
      const pivFormatted = decimals !== null 
        ? pivAmount.toFixed(decimals)
        : pivAmount.toFixed(8)
      return `${result} (${pivFormatted} PIV)`
    }

    return result
  }

  /**
   * Format amount with automatic currency from settings
   * @param {number} pivAmount - Amount in PIV
   * @returns {string} Formatted amount
   */
  const formatWithPreferred = (pivAmount) => {
    return formatAmount(pivAmount, {
      currency: settingsStore.preferredCurrency,
      showPIV: settingsStore.preferredCurrency !== 'PIV'
    })
  }

  /**
   * Get currency symbol for a currency code
   * @param {string} currency - Currency code (USD, EUR, BTC, PIV)
   * @returns {string} Currency symbol
   */
  const getCurrencySymbol = (currency) => {
    return settingsStore.getCurrencySymbol(currency)
  }

  /**
   * Format just the fiat value without PIV
   * @param {number} pivAmount - Amount in PIV
   * @param {string} currency - Target currency
   * @returns {string} Fiat amount only
   */
  const formatFiatOnly = (pivAmount, currency = null) => {
    const curr = currency || settingsStore.preferredCurrency
    return formatAmount(pivAmount, {
      currency: curr,
      showPIV: false,
      showSymbol: true
    })
  }

  /**
   * Check if fiat display is enabled (currency is not PIV)
   */
  const isFiatEnabled = computed(() => {
    return settingsStore.preferredCurrency !== 'PIV' && priceStore.hasValidPrices
  })

  /**
   * Get current price for the preferred currency
   */
  const currentPrice = computed(() => {
    const curr = settingsStore.preferredCurrency.toLowerCase()
    return priceStore.prices[curr] || 0
  })

  return {
    // Formatting functions
    formatAmount,
    formatWithPreferred,
    formatFiatOnly,
    getCurrencySymbol,
    // State
    preferredCurrency: computed(() => settingsStore.preferredCurrency),
    prices: computed(() => priceStore.prices),
    isFiatEnabled,
    currentPrice,
    hasValidPrices: computed(() => priceStore.hasValidPrices),
    isStale: computed(() => priceStore.isStale)
  }
}
