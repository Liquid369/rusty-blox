<template>
  <div
    class="sync-pill"
    :class="loading && !height ? 'pill-loading' : healthy ? 'pill-synced' : 'pill-syncing'"
    :title="title"
  >
    <span class="pill-dot"></span>
    <span v-if="loading && !height" class="pill-text">Connecting…</span>
    <span v-else-if="healthy" class="pill-text">
      Synced · <span class="pill-num">{{ formatNumber(height) }}</span>
    </span>
    <span v-else class="pill-text">
      Syncing · <span class="pill-num">{{ formatPercentage(syncPercentage) }}%</span>
    </span>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import { formatNumber, formatPercentage } from '@/utils/formatters'

const props = defineProps({
  healthy: {
    type: Boolean,
    default: false
  },
  height: {
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
  },
  loading: {
    type: Boolean,
    default: false
  }
})

const title = computed(() => {
  if (props.healthy) return `Fully synced at block ${formatNumber(props.height)}`
  return `${formatNumber(props.blocksBehind)} blocks behind network`
})
</script>

<style scoped>
.sync-pill {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-4);
  border-radius: var(--radius-full);
  background: var(--glass-bg-strong);
  border: 1px solid var(--glass-border);
  backdrop-filter: blur(var(--blur-sm));
  -webkit-backdrop-filter: blur(var(--blur-sm));
  font-size: var(--text-sm);
  font-weight: var(--weight-semibold);
  white-space: nowrap;
}

.pill-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}

.pill-text {
  color: var(--text-secondary);
}

.pill-num {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
  color: var(--text-primary);
}

.pill-synced {
  border-color: rgba(var(--rgb-green-accent), 0.35);
}

.pill-synced .pill-dot {
  background: var(--success);
  box-shadow:
    0 0 0 3px rgba(var(--rgb-green-accent), 0.15),
    0 0 10px rgba(var(--rgb-green-accent), 0.5);
  animation: pill-pulse 2.4s ease-in-out infinite;
}

.pill-synced .pill-num {
  color: var(--success);
}

.pill-syncing {
  border-color: rgba(246, 255, 120, 0.35);
}

.pill-syncing .pill-dot {
  background: var(--warning);
  box-shadow: 0 0 10px rgba(246, 255, 120, 0.45);
  animation: pill-pulse 1.2s ease-in-out infinite;
}

.pill-syncing .pill-num {
  color: var(--warning);
}

.pill-loading .pill-dot {
  background: var(--text-tertiary);
}

@keyframes pill-pulse {
  0%, 100% {
    opacity: 1;
  }
  50% {
    opacity: 0.45;
  }
}
</style>
