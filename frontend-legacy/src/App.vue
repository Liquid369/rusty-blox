<template>
  <div id="app">
    <!-- Reorg warning banner -->
    <div v-if="chainStore.reorgDetected" class="reorg-banner">
      <div class="reorg-content">
        <span class="reorg-icon">⚠️</span>
        <div class="reorg-text">
          <strong>Chain Reorganization Detected</strong>
          <span class="reorg-details">
            Block height {{ chainStore.lastReorgInfo?.oldHeight }} → {{ chainStore.lastReorgInfo?.newHeight }}
            - Data is being refreshed
          </span>
        </div>
        <button @click="chainStore.clearReorgFlag" class="reorg-dismiss">✕</button>
      </div>
    </div>
    
    <!-- WebSocket reconnect failure banner -->
    <div v-if="wsStore.anyReconnectFailed" class="ws-error-banner">
      <div class="ws-error-content">
        <span class="ws-error-icon">🔌</span>
        <div class="ws-error-text">
          <strong>Unable to Connect to Live Updates</strong>
          <span class="ws-error-details">
            Maximum reconnection attempts reached. Live block and transaction updates are unavailable.
          </span>
        </div>
        <button @click="wsStore.retryAll" class="ws-error-retry">Retry Connection</button>
      </div>
    </div>
    <RouterView />
  </div>
</template>

<script setup>
import { onMounted, onUnmounted } from 'vue'
import { useChainStore } from '@/stores/chainStore'
import { useWebSocketStore } from '@/stores/websocketStore'
import { usePriceStore } from '@/stores/priceStore'

const chainStore = useChainStore()
const wsStore = useWebSocketStore()
const priceStore = usePriceStore()

let priceInterval = null

onMounted(() => {
  // Initialize chain state on app mount
  chainStore.fetchChainState()
  
  // Refresh chain state every 10 seconds
  setInterval(() => {
    chainStore.fetchChainState()
  }, 10000)
  
  // Start price auto-refresh (60 second interval)
  priceInterval = priceStore.startAutoRefresh()
})

onUnmounted(() => {
  // Cleanup price interval
  if (priceInterval) {
    clearInterval(priceInterval)
  }
})
</script>

<style>
#app {
  min-height: 100vh;
  display: flex;
  flex-direction: column;
}

.reorg-banner {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  background: linear-gradient(135deg, #ff6b6b 0%, #ee5a24 100%);
  color: white;
  padding: var(--space-3) var(--space-4);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.2);
  z-index: 10000;
  animation: slideDown 0.3s ease-out;
}

@keyframes slideDown {
  from {
    transform: translateY(-100%);
    opacity: 0;
  }
  to {
    transform: translateY(0);
    opacity: 1;
  }
}

.reorg-content {
  max-width: 1200px;
  margin: 0 auto;
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

.reorg-icon {
  font-size: 24px;
  flex-shrink: 0;
}

.reorg-text {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.reorg-text strong {
  font-size: var(--text-base);
  font-weight: 600;
}

.reorg-details {
  font-size: var(--text-sm);
  opacity: 0.95;
}

.reorg-dismiss {
  background: rgba(255, 255, 255, 0.2);
  border: 1px solid rgba(255, 255, 255, 0.3);
  color: white;
  padding: var(--space-2);
  border-radius: var(--radius-md);
  cursor: pointer;
  font-size: 18px;
  line-height: 1;
  transition: background 0.2s;
  flex-shrink: 0;
}

.reorg-dismiss:hover {
  background: rgba(255, 255, 255, 0.3);
}

/* WebSocket Error Banner */
.ws-error-banner {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  background: linear-gradient(135deg, #f39c12 0%, #e67e22 100%);
  color: white;
  padding: var(--space-3) var(--space-4);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.2);
  z-index: 9999;
  animation: slideDown 0.3s ease-out;
}

.ws-error-content {
  max-width: 1200px;
  margin: 0 auto;
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

.ws-error-icon {
  font-size: 24px;
  flex-shrink: 0;
}

.ws-error-text {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.ws-error-text strong {
  font-size: var(--text-base);
  font-weight: 600;
}

.ws-error-details {
  font-size: var(--text-sm);
  opacity: 0.95;
}

.ws-error-retry {
  background: rgba(255, 255, 255, 0.9);
  border: none;
  color: #e67e22;
  padding: var(--space-2) var(--space-4);
  border-radius: var(--radius-md);
  cursor: pointer;
  font-size: var(--text-sm);
  font-weight: 600;
  transition: all 0.2s;
  flex-shrink: 0;
}

.ws-error-retry:hover {
  background: white;
  transform: translateY(-1px);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.15);
}
</style>
