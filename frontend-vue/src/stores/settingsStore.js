import { defineStore } from 'pinia'
import { ref } from 'vue'

export const useSettingsStore = defineStore('settings', () => {
  const isDarkMode = ref(true)
  const itemsPerPage = ref(25)
  const enableAnimations = ref(true)

  // Load from localStorage on init
  const init = () => {
    const savedTheme = localStorage.getItem('pivx-theme')
    if (savedTheme) {
      isDarkMode.value = savedTheme === 'dark'
    }

    const savedItemsPerPage = localStorage.getItem('pivx-items-per-page')
    if (savedItemsPerPage) {
      itemsPerPage.value = parseInt(savedItemsPerPage, 10)
    }
  }

  const toggleTheme = () => {
    isDarkMode.value = !isDarkMode.value
    localStorage.setItem('pivx-theme', isDarkMode.value ? 'dark' : 'light')
  }

  const setItemsPerPage = (value) => {
    itemsPerPage.value = value
    localStorage.setItem('pivx-items-per-page', value.toString())
  }

  init()

  return {
    isDarkMode,
    itemsPerPage,
    enableAnimations,
    toggleTheme,
    setItemsPerPage
  }
})
