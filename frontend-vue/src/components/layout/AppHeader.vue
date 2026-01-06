<template>
  <header class="app-header">
    <div class="header-container">
      <router-link to="/" class="logo-link">
        <img src="/PIVX-Horz-White.svg" alt="PIVX" class="logo" />
      </router-link>

      <nav class="nav-primary hide-mobile">
        <router-link to="/" class="nav-link">Dashboard</router-link>
        <router-link to="/blocks" class="nav-link">Blocks</router-link>
        <router-link to="/mempool" class="nav-link">Mempool</router-link>
        <router-link to="/masternodes" class="nav-link">Masternodes</router-link>
        <router-link to="/governance" class="nav-link">Governance</router-link>
        <router-link to="/analytics" class="nav-link">Analytics</router-link>
      </nav>

      <div class="header-actions">
        <SearchBar />
        <button @click="toggleTheme" class="theme-toggle" aria-label="Toggle theme">
          {{ isDarkMode ? '☀️' : '🌙' }}
        </button>
      </div>
    </div>

    <div class="sync-status" v-if="!isSynced">
      <span class="sync-indicator"></span>
      Syncing: {{ syncPercentage.toFixed(2) }}%
    </div>
  </header>
</template>

<script setup>
import { computed } from 'vue'
import { useSettingsStore } from '@/stores/settingsStore'
import { useChainStore } from '@/stores/chainStore'
import SearchBar from './SearchBar.vue'

const settingsStore = useSettingsStore()
const chainStore = useChainStore()

const isDarkMode = computed(() => settingsStore.isDarkMode)
const isSynced = computed(() => chainStore.isSynced)
const syncPercentage = computed(() => chainStore.syncPercentage)

const toggleTheme = () => {
  settingsStore.toggleTheme()
}
</script>

<style scoped>
.app-header {
  background: var(--bg-secondary);
  border-bottom: 2px solid var(--border-primary);
  position: sticky;
  top: 0;
  z-index: 100;
}

.header-container {
  max-width: 1200px;
  margin: 0 auto;
  padding: var(--space-4) var(--space-6);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-6);
}

.logo {
  height: 32px;
  width: auto;
}

.logo-link {
  display: flex;
  align-items: center;
}

.nav-primary {
  display: flex;
  gap: var(--space-6);
}

.nav-link {
  font-size: var(--text-base);
  font-weight: var(--weight-bold);
  color: var(--text-secondary);
  transition: color var(--transition-fast);
}

.nav-link:hover,
.nav-link.router-link-active {
  color: var(--text-accent);
}

.header-actions {
  display: flex;
  align-items: center;
  gap: var(--space-4);
}

.theme-toggle {
  width: 44px;
  height: 44px;
  border: 2px solid var(--border-primary);
  background: transparent;
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: 20px;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all var(--transition-base);
}

.theme-toggle:hover {
  background: var(--bg-tertiary);
  border-color: var(--border-accent);
}

.sync-status {
  background: var(--warning);
  color: var(--bg-primary);
  padding: var(--space-2) var(--space-6);
  text-align: center;
  font-size: var(--text-sm);
  font-weight: var(--weight-bold);
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
}

.sync-indicator {
  width: 8px;
  height: 8px;
  background: var(--bg-primary);
  border-radius: 50%;
  animation: pulse 1.5s ease-in-out infinite;
}

@media (max-width: 768px) {
  .header-container {
    flex-wrap: wrap;
  }

  .logo {
    height: 24px;
  }

  .header-actions {
    order: 3;
    width: 100%;
    margin-top: var(--space-2);
  }
}
</style>
