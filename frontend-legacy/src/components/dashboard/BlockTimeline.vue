<template>
  <div class="timeline-wrap">
    <!-- Loading skeletons -->
    <div v-if="loading" class="timeline-strip">
      <div v-for="i in 6" :key="`sk-${i}`" class="tile-skeleton">
        <SkeletonLoader variant="card" height="100%" />
      </div>
    </div>

    <!-- Error state -->
    <div v-else-if="error" class="timeline-error">
      <p><Icon name="alert-triangle" :size="14" /> Failed to load recent blocks</p>
    </div>

    <!-- Block strip -->
    <div v-else class="timeline-strip">
      <!-- Pending (next block) tile -->
      <router-link to="/mempool" class="tile tile-pending">
        <div class="tile-height pending-label">
          <span class="pending-dot"></span>
          Pending
        </div>
        <div class="tile-rows">
          <div class="tile-row">
            <span class="tile-row-value">{{ formatNumber(pending.txCount) }}</span>
            <span class="tile-row-label">{{ pending.txCount === 1 ? 'tx' : 'txs' }}</span>
          </div>
          <div class="tile-row">
            <span class="tile-row-value">{{ pendingKb }}</span>
            <span class="tile-row-label">KB</span>
          </div>
        </div>
        <div class="tile-staker pending-sub">in mempool</div>
      </router-link>

      <div class="tile-divider" aria-hidden="true"></div>

      <TransitionGroup name="tile-enter">
        <router-link
          v-for="block in blocks"
          :key="block.height"
          :to="`/block/${block.height}`"
          class="tile"
        >
          <div class="tile-height">#{{ formatNumber(block.height) }}</div>
          <div class="tile-time">{{ timeAgo(block.time) }}</div>
          <div class="tile-rows">
            <div class="tile-row">
              <span class="tile-row-value">{{ formatNumber(block.txCount) }}</span>
              <span class="tile-row-label">{{ block.txCount === 1 ? 'tx' : 'txs' }}</span>
            </div>
            <div class="tile-row">
              <span class="tile-row-value">{{ sizeKb(block.size) }}</span>
              <span class="tile-row-label">KB</span>
            </div>
          </div>
          <div class="tile-staker" :title="block.staker || ''">
            <span class="staker-icon"><Icon name="zap" :size="12" /></span>
            {{ block.staker ? truncateHash(block.staker, 6, 4) : '—' }}
          </div>
        </router-link>
      </TransitionGroup>
    </div>
  </div>
</template>

<script setup>
import Icon from '@/components/common/Icon.vue'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { formatNumber, formatTimeAgo, truncateHash } from '@/utils/formatters'
import SkeletonLoader from '@/components/common/SkeletonLoader.vue'

const props = defineProps({
  blocks: {
    type: Array,
    default: () => []
  },
  pending: {
    type: Object,
    default: () => ({ txCount: 0, bytes: 0 })
  },
  loading: {
    type: Boolean,
    default: false
  },
  error: {
    type: Boolean,
    default: false
  }
})

// Ticker so "time ago" labels stay fresh between new blocks
const tick = ref(0)
let tickTimer = null

onMounted(() => {
  tickTimer = setInterval(() => {
    tick.value++
  }, 10000)
})

onUnmounted(() => {
  if (tickTimer) clearInterval(tickTimer)
})

const timeAgo = (timestamp) => {
  // referencing tick makes labels recompute every 10s
  void tick.value
  return formatTimeAgo(timestamp)
}

const sizeKb = (bytes) => {
  if (!bytes || isNaN(bytes)) return '0.0'
  return (bytes / 1024).toFixed(1)
}

const pendingKb = computed(() => sizeKb(props.pending?.bytes))
</script>

<style scoped>
.timeline-wrap {
  width: 100%;
}

.timeline-strip {
  display: flex;
  align-items: stretch;
  gap: var(--space-3);
  overflow-x: auto;
  padding: var(--space-2) var(--space-1) var(--space-4);
  scrollbar-width: thin;
  scrollbar-color: var(--purple-mid) transparent;
}

.timeline-strip::-webkit-scrollbar {
  height: 6px;
}

.timeline-strip::-webkit-scrollbar-thumb {
  background: var(--purple-mid);
  border-radius: var(--radius-full);
}

.tile,
.tile-skeleton {
  flex: 0 0 132px;
  width: 132px;
  min-height: 132px;
}

.tile {
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-4);
  border-radius: var(--radius-md);
  background: var(--glass-bg);
  border: 1px solid var(--glass-border);
  backdrop-filter: blur(var(--blur-sm));
  -webkit-backdrop-filter: blur(var(--blur-sm));
  box-shadow: var(--shadow-xs), var(--glass-highlight);
  text-decoration: none;
  transition:
    transform var(--transition-base),
    border-color var(--transition-base),
    box-shadow var(--transition-base);
}

.tile:hover {
  transform: translateY(-3px);
  border-color: var(--glass-border-hover);
  box-shadow: var(--shadow-md), var(--glow-purple), var(--glass-highlight);
}

.tile:focus-visible {
  outline: 2px solid var(--focus-ring-color);
  outline-offset: 2px;
}

.tile-height {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
  font-size: var(--text-sm);
  font-weight: var(--weight-bold);
  color: var(--text-accent);
  white-space: nowrap;
}

.tile-time {
  font-size: var(--text-xs);
  color: var(--text-tertiary);
  font-variant-numeric: tabular-nums;
}

.tile-rows {
  display: grid;
  gap: 2px;
}

.tile-row {
  display: flex;
  align-items: baseline;
  gap: var(--space-1);
}

.tile-row-value {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
  font-size: var(--text-base);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
}

.tile-row-label {
  font-size: var(--text-2xs);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
  color: var(--text-tertiary);
}

.tile-staker {
  font-family: var(--font-mono);
  font-size: var(--text-2xs);
  color: var(--text-purple);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.staker-icon {
  margin-right: 2px;
  opacity: 0.8;
}

/* Pending tile */
.tile-pending {
  border-style: dashed;
  border-color: rgba(var(--rgb-green-accent), 0.4);
  background: rgba(var(--rgb-purple-darkest), 0.5);
  animation: pending-pulse 2.4s ease-in-out infinite;
}

.tile-pending:hover {
  border-color: rgba(var(--rgb-green-accent), 0.7);
  box-shadow: var(--shadow-md), var(--glow-green), var(--glass-highlight);
}

.pending-label {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  color: var(--success);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
  font-family: var(--font-primary);
  font-size: var(--text-xs);
}

.pending-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--success);
  box-shadow: 0 0 8px rgba(var(--rgb-green-accent), 0.6);
  animation: pending-dot-pulse 1.4s ease-in-out infinite;
  flex-shrink: 0;
}

.pending-sub {
  color: var(--text-tertiary);
  font-family: var(--font-primary);
  font-style: italic;
}

.tile-divider {
  flex: 0 0 1px;
  align-self: stretch;
  background: linear-gradient(180deg, transparent, rgba(var(--rgb-purple-accent), 0.5), transparent);
}

@keyframes pending-pulse {
  0%, 100% {
    border-color: rgba(var(--rgb-green-accent), 0.4);
  }
  50% {
    border-color: rgba(var(--rgb-green-accent), 0.15);
  }
}

@keyframes pending-dot-pulse {
  0%, 100% {
    opacity: 1;
  }
  50% {
    opacity: 0.35;
  }
}

/* New block entrance animation */
.tile-enter-enter-active {
  animation: tile-slide-in 500ms var(--ease-out);
}

.tile-enter-leave-active {
  display: none;
}

.tile-enter-move {
  transition: transform 400ms var(--ease-out);
}

@keyframes tile-slide-in {
  0% {
    opacity: 0;
    transform: translateX(-24px) scale(0.92);
  }
  60% {
    box-shadow: var(--shadow-md), var(--glow-green-strong), var(--glass-highlight);
  }
  100% {
    opacity: 1;
    transform: translateX(0) scale(1);
  }
}

.timeline-error {
  text-align: center;
  color: var(--text-tertiary);
  font-style: italic;
  padding: var(--space-6);
}

@media (max-width: 768px) {
  .tile,
  .tile-skeleton {
    flex: 0 0 120px;
    width: 120px;
    min-height: 124px;
  }
}
</style>
