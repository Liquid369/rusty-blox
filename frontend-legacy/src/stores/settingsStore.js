import { defineStore } from 'pinia'
import { ref } from 'vue'

export const useSettingsStore = defineStore('settings', () => {
  // State
  const theme = ref('dark')
  const searchHistory = ref([])
  const maxSearchHistory = 10
  const itemsPerPage = ref(25)
  const preferredCurrency = ref('PIV')
  const showTestnet = ref(false)

  // Load from localStorage on init
  const loadSettings = () => {
    try {
      const saved = localStorage.getItem('pivx-explorer-settings')
      if (saved) {
        const settings = JSON.parse(saved)
        theme.value = settings.theme || 'dark'
        searchHistory.value = settings.searchHistory || []
        itemsPerPage.value = settings.itemsPerPage || 25
        preferredCurrency.value = settings.preferredCurrency || 'PIV'
        showTestnet.value = settings.showTestnet || false
      }
    } catch (err) {
      console.error('Failed to load settings:', err)
    }
  }

  // Save to localStorage
  const saveSettings = () => {
    try {
      const settings = {
        theme: theme.value,
        searchHistory: searchHistory.value,
        itemsPerPage: itemsPerPage.value,
        preferredCurrency: preferredCurrency.value,
        showTestnet: showTestnet.value
      }
      localStorage.setItem('pivx-explorer-settings', JSON.stringify(settings))
    } catch (err) {
      console.error('Failed to save settings:', err)
    }
  }

  // Actions
  const setTheme = (newTheme) => {
    theme.value = newTheme
    saveSettings()
  }

  const addToSearchHistory = (query) => {
    // Remove duplicates
    searchHistory.value = searchHistory.value.filter(item => item !== query)
    // Add to beginning
    searchHistory.value.unshift(query)
    // Limit size
    if (searchHistory.value.length > maxSearchHistory) {
      searchHistory.value = searchHistory.value.slice(0, maxSearchHistory)
    }
    saveSettings()
  }

  const clearSearchHistory = () => {
    searchHistory.value = []
    saveSettings()
  }

  const setItemsPerPage = (count) => {
    itemsPerPage.value = count
    saveSettings()
  }

  const $reset = () => {
    theme.value = 'dark'
    searchHistory.value = []
    itemsPerPage.value = 25
    preferredCurrency.value = 'PIV'
    showTestnet.value = false
    saveSettings()
  }

  // Load settings on store initialization
  loadSettings()

  return {
    // State
    theme,
    searchHistory,
    itemsPerPage,
    preferredCurrency,
    showTestnet,
    // Actions
    setTheme,
    addToSearchHistory,
    clearSearchHistory,
    setItemsPerPage,
    loadSettings,
    saveSettings,
    $reset
  }
})
