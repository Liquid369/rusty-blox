<template>
  <header class="app-header">
    <div class= "header-container">
      <!-- Logo and Title -->
      <RouterLink to="/" class="logo-section">
        <img 
          src="/PIVX-Horz-White.svg" 
          alt="PIVX Logo" 
          class="logo"
          @error="onLogoError"
        >
        <div class="title-section">
        </div>
      </RouterLink>

      <!-- Search Bar -->
      <SearchBar class="header-search" />

      <!-- Price Widget -->
      <PriceCard class="header-price" />

      <!-- Navigation -->
      <nav class="main-nav">
        <RouterLink to="/" class="nav-link">Dashboard</RouterLink>
        <RouterLink to="/blocks" class="nav-link">Blocks</RouterLink>
        <RouterLink to="/mempool" class="nav-link">Mempool</RouterLink>
        <RouterLink to="/masternodes" class="nav-link">Masternodes</RouterLink>
        <RouterLink to="/governance" class="nav-link">Governance</RouterLink>
        <RouterLink to="/analytics" class="nav-link">Analytics</RouterLink>
      </nav>
    </div>

    <!-- Sync Progress Bar -->
    <div v-if="chainStore.isSyncing" class="sync-progress">
      <div class="progress-bar">
        <div 
          class="progress-fill" 
          :style="{ width: chainStore.syncPercentage + '%' }"
        ></div>
      </div>
      <div class="progress-text">
        Syncing: {{ formatNumber(chainStore.syncHeight) }} / {{ formatNumber(chainStore.networkHeight) }}
        ({{ formatPercentage(chainStore.syncPercentage) }}%)
      </div>
    </div>
  </header>
</template>

<script setup>
import { onMounted } from 'vue'
import { useChainStore } from '@/stores/chainStore'
import { useWebSocketStore } from '@/stores/websocketStore'
import { formatNumber, formatPercentage } from '@/utils/formatters'
import SearchBar from './SearchBar.vue'
import LiveIndicator from '@/components/common/LiveIndicator.vue'
import PriceCard from '@/components/common/PriceCard.vue'

const chainStore = useChainStore()
const wsStore = useWebSocketStore()

const onLogoError = (e) => {
  e.target.style.display = 'none'
}

// Connect WebSockets when app loads
onMounted(() => {
  wsStore.connectAll()
})
</script>

<style scoped>
.app-header {
  background: linear-gradient(180deg, var(--purple-mid), var(--purple-main));
  border-bottom: 0px solid var(--border-primary);
  padding: var(--space-4) 0;
  position: sticky;
  top: 0;
  z-index: var(--z-sticky);
  backdrop-filter: blur(10px);
  box-shadow: 0 0 20px #00000050;
}

.header-container {
  max-width: 1200px;
  margin: 0 auto;
  padding: 0 var(--space-6);
  display: flex;
  align-items: center;
  gap: var(--space-6);
  flex-wrap: wrap;
  justify-content: center;
}

.logo-section {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  text-decoration: none;
  transition: opacity var(--transition-fast);
}

.logo-section:hover {
  opacity: 0.8;
}

.logo {
  height: 60px;
  width: auto;
  display: block;
}

.title-section {
  display: flex;
  flex-direction: column;
  justify-content: center;
  padding-top: 14px;
}

.site-title {
  font-size: var(--text-3xl);
  font-weight: var(--weight-extrabold);
  color: var(--text-primary);
  line-height: 1;
  margin: 0;
  padding: 0;
}

.site-subtitle {
  font-size: var(--text-xs);
  color: var(--text-tertiary);
  font-weight: var(--weight-bold);
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.network-status {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  background: var(--bg-tertiary);
  padding: var(--space-2) var(--space-4);
  border-radius: var(--radius-lg);
  border: 2px solid var(--border-secondary);
  font-size: var(--text-sm);
}

.status-indicator {
  flex-shrink: 0;
}

.status-info {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.status-text {
  color: var(--text-secondary);
  font-weight: var(--weight-bold);
}

.status-height {
  color: var(--text-primary);
  font-family: var(--font-mono);
  font-weight: var(--weight-bold);
}

.header-search {
  flex: 1;
  min-width: 250px;
  max-width: 500px;
}

.header-price {
  flex-shrink: 0;
}

.main-nav {
  display: flex;
  gap: var(--space-2);
  margin-center: auto;
}

.nav-link {
  color: var(--text-secondary);
  text-decoration: none;
  padding: var(--space-2) var(--space-4);
  border-radius: var(--radius-sm);
  font-weight: var(--weight-bold);
  font-size: var(--text-sm);
  transition: all var(--transition-fast);
  white-space: nowrap;
}

.nav-link:hover {
  background: var(--purple-mid);
  color: var(--text-primary);
}

.nav-link.router-link-active {
  background: var(--purple-accent);
  color: var(--text-primary);
}

.sync-progress {
  margin-top: var(--space-4);
  padding: 0 var(--space-6);
}

.progress-bar {
  width: 100%;
  height: 4px;
  background: var(--purple-dark);
  border-radius: 2px;
  overflow: hidden;
}

.progress-fill {
  height: 100%;
  background: linear-gradient(90deg, var(--purple-main), var(--purple-accent));
  transition: width var(--transition-slow);
  box-shadow: var(--shadow-glow);
}

.progress-text {
  margin-top: var(--space-2);
  font-size: var(--text-xs);
  color: var(--text-tertiary);
  text-align: center;
}

/* Mobile Responsive */
@media (max-width: 1024px) {
  .main-nav {
    display: none;
  }
}

@media (max-width: 768px) {
  .header-container {
    padding: 0 var(--space-4);
    gap: var(--space-4);
  }

  .logo {
    height: 32px;
  }

  .site-title {
    font-size: var(--text-xl);
  }

  .header-search {
    flex-basis: 100%;
    max-width: 100%;
  }

  .network-status {
    font-size: var(--text-xs);
    padding: var(--space-1) var(--space-3);
  }

  .sync-progress {
    padding: 0 var(--space-4);
  }
}
</style>
