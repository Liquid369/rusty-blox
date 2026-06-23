<template>
  <header class="app-header">
    <div class="header-container">
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

      <!-- Mobile Menu Toggle (shown <=1024px) -->
      <button
        type="button"
        class="nav-toggle"
        aria-label="Open navigation menu"
        :aria-expanded="mobileMenuOpen"
        aria-controls="mobile-menu"
        aria-haspopup="dialog"
        @click="mobileMenuOpen = true"
      >
        <svg
          width="24"
          height="24"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1.75"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <path d="M4 6h16 M4 12h16 M4 18h16" />
        </svg>
      </button>
    </div>

    <!-- Mobile Drawer -->
    <MobileMenu
      id="mobile-menu"
      :open="mobileMenuOpen"
      @close="mobileMenuOpen = false"
    />

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
import { onMounted, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { useChainStore } from '@/stores/chainStore'
import { useWebSocketStore } from '@/stores/websocketStore'
import { formatNumber, formatPercentage } from '@/utils/formatters'
import SearchBar from './SearchBar.vue'
import MobileMenu from './MobileMenu.vue'
import PriceCard from '@/components/common/PriceCard.vue'

const chainStore = useChainStore()
const wsStore = useWebSocketStore()
const route = useRoute()

const mobileMenuOpen = ref(false)

// Safety net: close the drawer on any route change.
watch(() => route.fullPath, () => {
  mobileMenuOpen.value = false
})

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
  background: linear-gradient(
    180deg,
    rgba(var(--rgb-purple-mid), 0.92),
    rgba(var(--rgb-purple-main), 0.82)
  );
  border-bottom: 1px solid var(--glass-border);
  padding: var(--space-4) 0;
  position: sticky;
  top: 0;
  z-index: var(--z-sticky);
  backdrop-filter: blur(var(--blur-md));
  -webkit-backdrop-filter: blur(var(--blur-md));
  box-shadow: var(--shadow-md);
}

.header-container {
  max-width: 1200px;
  margin: 0 auto;
  padding: 0 var(--space-6);
  display: flex;
  align-items: center;
  gap: var(--space-4) var(--space-6);
  flex-wrap: wrap;
  justify-content: flex-start;
}

.logo-section {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-shrink: 0;
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
  gap: var(--space-1);
  margin-left: auto;
}

/* Mobile menu toggle — hidden on desktop, revealed <=1024px */
.nav-toggle {
  display: none;
  align-items: center;
  justify-content: center;
  width: 44px;
  height: 44px;
  flex-shrink: 0;
  margin-left: auto;
  background: rgba(var(--rgb-purple-darkest), 0.45);
  border: 1px solid var(--glass-border);
  border-radius: var(--radius-full);
  color: var(--text-secondary);
  cursor: pointer;
  transition:
    background-color var(--transition-fast),
    border-color var(--transition-fast),
    color var(--transition-fast),
    box-shadow var(--transition-fast);
}

.nav-toggle:hover {
  background: rgba(var(--rgb-purple-accent), 0.3);
  border-color: var(--glass-border-hover);
  color: var(--text-primary);
}

.nav-toggle:focus-visible {
  outline: 2px solid var(--green-accent);
  outline-offset: 2px;
  box-shadow: var(--glow-green);
}

.nav-link {
  color: rgba(255, 255, 255, 0.78);
  text-decoration: none;
  padding: var(--space-2) var(--space-4);
  border-radius: var(--radius-full);
  border: 1px solid transparent;
  font-weight: var(--weight-semibold);
  font-size: var(--text-sm);
  letter-spacing: var(--tracking-wide);
  transition:
    background-color var(--transition-fast),
    border-color var(--transition-fast),
    color var(--transition-fast),
    box-shadow var(--transition-fast);
  white-space: nowrap;
}

.nav-link:hover {
  background: rgba(var(--rgb-purple-darkest), 0.35);
  color: var(--text-primary);
}

.nav-link:focus-visible {
  outline: 2px solid var(--green-accent);
  outline-offset: 2px;
  box-shadow: var(--glow-green);
}

.nav-link.router-link-active {
  background: rgba(var(--rgb-purple-darkest), 0.5);
  border-color: rgba(var(--rgb-green-accent), 0.45);
  color: var(--green-accent);
  box-shadow: var(--glow-green);
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
  background: linear-gradient(90deg, var(--purple-accent), var(--green-accent));
  transition: width var(--transition-slow);
  box-shadow: var(--glow-green);
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

  .nav-toggle {
    display: flex;
  }

  .header-container {
    justify-content: space-between;
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
