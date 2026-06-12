<template>
  <div class="sync-card">
    <div class="sync-card-top">
      <div class="sync-card-title">
        <span class="sync-spinner"></span>
        Indexing the blockchain
      </div>
      <div class="sync-card-pct">{{ formatPercentage(syncPercentage) }}%</div>
    </div>

    <div
      class="sync-bar"
      role="progressbar"
      :aria-valuenow="Math.round(syncPercentage)"
      aria-valuemin="0"
      aria-valuemax="100"
    >
      <div class="sync-bar-fill" :style="{ width: barWidth }"></div>
    </div>

    <div class="sync-card-meta">
      <div class="sync-meta-item">
        <span class="sync-meta-label">Indexed</span>
        <span class="sync-meta-value">{{ formatNumber(syncHeight) }}</span>
      </div>
      <div class="sync-meta-item">
        <span class="sync-meta-label">Network</span>
        <span class="sync-meta-value">{{ formatNumber(networkHeight) }}</span>
      </div>
      <div class="sync-meta-item">
        <span class="sync-meta-label">Behind</span>
        <span class="sync-meta-value behind">{{ formatNumber(blocksBehind) }} blocks</span>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import { formatNumber, formatPercentage } from '@/utils/formatters'

const props = defineProps({
  syncHeight: {
    type: Number,
    default: 0
  },
  networkHeight: {
    type: Number,
    default: 0
  },
  syncPercentage: {
    type: Number,
    default: 0
  },
  blocksBehind: {
    type: Number,
    default: 0
  }
})

const barWidth = computed(() => {
  const pct = Math.min(100, Math.max(0, props.syncPercentage))
  return `${pct}%`
})
</script>

<style scoped>
.sync-card {
  background: var(--glass-bg);
  border: 1px solid rgba(246, 255, 120, 0.3);
  border-radius: var(--radius-lg);
  backdrop-filter: blur(var(--blur-md));
  -webkit-backdrop-filter: blur(var(--blur-md));
  box-shadow: var(--shadow-sm), var(--glass-highlight);
  padding: var(--space-6);
  display: grid;
  gap: var(--space-4);
}

.sync-card-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
}

.sync-card-title {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  font-size: var(--text-lg);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
}

.sync-spinner {
  width: 14px;
  height: 14px;
  border-radius: 50%;
  border: 2px solid rgba(246, 255, 120, 0.25);
  border-top-color: var(--warning);
  animation: sync-spin 0.9s linear infinite;
  flex-shrink: 0;
}

.sync-card-pct {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
  font-size: var(--text-2xl);
  font-weight: var(--weight-bold);
  color: var(--warning);
  text-shadow: 0 0 20px rgba(246, 255, 120, 0.3);
}

.sync-bar {
  height: 10px;
  border-radius: var(--radius-full);
  background: rgba(var(--rgb-purple-darkest), 0.7);
  border: 1px solid var(--border-subtle);
  overflow: hidden;
}

.sync-bar-fill {
  height: 100%;
  border-radius: var(--radius-full);
  background: linear-gradient(90deg, var(--purple-accent), var(--green-accent));
  box-shadow: 0 0 12px rgba(var(--rgb-green-accent), 0.4);
  transition: width 600ms var(--ease-out);
}

.sync-card-meta {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-6);
}

.sync-meta-item {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.sync-meta-label {
  font-size: var(--text-2xs);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
  color: var(--text-tertiary);
  font-weight: var(--weight-bold);
}

.sync-meta-value {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
  font-size: var(--text-base);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
}

.sync-meta-value.behind {
  color: var(--warning);
}

@keyframes sync-spin {
  to {
    transform: rotate(360deg);
  }
}

@media (max-width: 768px) {
  .sync-card {
    padding: var(--space-4);
  }

  .sync-card-meta {
    gap: var(--space-4);
  }
}
</style>
