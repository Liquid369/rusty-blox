<template>
  <div class="live-indicator" :class="statusClass" :title="statusText">
    <div class="indicator-dot"></div>
    <span v-if="showLabel" class="indicator-label">{{ statusText }}</span>
  </div>
</template>

<script setup>
import { computed } from 'vue'

const props = defineProps({
  connected: {
    type: Boolean,
    required: true
  },
  connecting: {
    type: Boolean,
    default: false
  },
  showLabel: {
    type: Boolean,
    default: false
  }
})

const statusClass = computed(() => {
  if (props.connected) return 'status-connected'
  if (props.connecting) return 'status-connecting'
  return 'status-disconnected'
})

const statusText = computed(() => {
  if (props.connected) return 'Live'
  if (props.connecting) return 'Connecting...'
  return 'Disconnected'
})
</script>

<style scoped>
.live-indicator {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--text-sm);
  font-weight: var(--weight-medium);
}

.indicator-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  transition: background-color var(--transition-slow), box-shadow var(--transition-slow);
}

/* Connected state - green with pulse + glow */
.status-connected .indicator-dot {
  background-color: var(--success);
  box-shadow:
    0 0 0 3px rgba(var(--rgb-green-accent), 0.15),
    0 0 10px rgba(var(--rgb-green-accent), 0.5);
  animation: pulse 2s ease-in-out infinite;
}

.status-connected .indicator-label {
  color: var(--success);
}

/* Connecting state - yellow with pulse */
.status-connecting .indicator-dot {
  background-color: var(--warning);
  box-shadow: 0 0 8px rgba(246, 255, 120, 0.4);
  animation: pulse 1s ease-in-out infinite;
}

.status-connecting .indicator-label {
  color: var(--warning);
}

/* Disconnected state - red, no pulse */
.status-disconnected .indicator-dot {
  background-color: var(--danger);
}

.status-disconnected .indicator-label {
  color: var(--danger);
}

@keyframes pulse {
  0%, 100% {
    opacity: 1;
    transform: scale(1);
  }
  50% {
    opacity: 0.5;
    transform: scale(1.2);
  }
}

/* Hover effect */
.live-indicator {
  cursor: help;
}

.live-indicator:hover .indicator-dot {
  transform: scale(1.3);
}
</style>
