<template>
  <div id="app" :class="{ 'theme-light': !isDarkMode }">
    <router-view />
  </div>
</template>

<script setup>
import { computed } from 'vue'
import { useSettingsStore } from '@/stores/settingsStore'

const settingsStore = useSettingsStore()
const isDarkMode = computed(() => settingsStore.isDarkMode)
</script>

<style>
#app {
  min-height: 100vh;
  background: var(--bg-primary);
  color: var(--text-primary);
  transition: background-color 0.3s ease, color 0.3s ease;
}
</style>
