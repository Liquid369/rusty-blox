<template>
  <Card
    :clickable="clickable"
    :hover="clickable"
    class="block-card"
    @click="handleClick"
  >
    <div class="block-card-content">
      <!-- Block Height -->
      <div class="block-height">
        <span class="block-label">Block</span>
        <span class="block-number">#{{ formatNumber(block.height) }}</span>
      </div>

      <!-- Block Info -->
      <div class="block-info">
        <div class="info-row">
          <span class="info-label">Hash</span>
          <span class="info-value font-mono">
            {{ truncateHash(block.hash, 10, 10) }}
            <CopyButton v-if="showCopy" :text="block.hash" class="copy-btn" />
          </span>
        </div>

        <div class="info-row">
          <span class="info-label">Transactions</span>
          <Badge variant="accent" size="sm">{{ block.txCount || 0 }} tx</Badge>
        </div>

        <div class="info-row">
          <span class="info-label">Time</span>
          <span class="info-value">{{ formatTimeAgo(block.time) }}</span>
        </div>

        <div v-if="block.size" class="info-row">
          <span class="info-label">Size</span>
          <span class="info-value">{{ formatBytes(block.size) }}</span>
        </div>
      </div>

      <!-- Block Type Badge -->
      <div v-if="block.isPoS !== undefined" class="block-type">
        <Badge :variant="block.isPoS ? 'success' : 'info'" size="sm">
          {{ block.isPoS ? 'PoS' : 'PoW' }}
        </Badge>
      </div>
    </div>
  </Card>
</template>

<script setup>
import { formatNumber, formatTimeAgo, truncateHash, formatBytes } from '@/utils/formatters'
import Card from './Card.vue'
import Badge from './Badge.vue'
import CopyButton from './CopyButton.vue'

const emit = defineEmits(['click'])

const props = defineProps({
  block: {
    type: Object,
    required: true
  },
  clickable: {
    type: Boolean,
    default: true
  },
  showCopy: {
    type: Boolean,
    default: true
  }
})

const handleClick = () => {
  if (props.clickable) {
    emit('click', props.block)
  }
}
</script>

<style scoped>
.block-card {
  transition: all var(--transition-base);
  position: relative;
}

.block-card::after {
  content: '';
  position: absolute;
  top: 0;
  right: 0;
  width: 100%;
  height: 100%;
  background: linear-gradient(135deg, transparent 0%, rgba(89, 252, 179, 0.05) 100%);
  opacity: 0;
  transition: opacity var(--transition-base);
  pointer-events: none;
  border-radius: var(--radius-lg);
}

.block-card:hover::after {
  opacity: 1;
}

.block-card-content {
  display: grid;
  gap: var(--space-4);
  position: relative;
  z-index: 1;
}

.block-height {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding-bottom: var(--space-4);
  border-bottom: 2px solid var(--border-subtle);
}

.block-label {
  font-size: var(--text-sm);
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 1px;
  font-weight: var(--weight-bold);
}

.block-number {
  font-size: var(--text-2xl);
  font-weight: var(--weight-extrabold);
  color: var(--pivx-accent);
  font-family: var(--font-mono);
  text-shadow: 0 0 10px rgba(89, 252, 179, 0.3);
}

.block-info {
  display: grid;
  gap: var(--space-3);
}

.info-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-2) 0;
}

.info-label {
  font-size: var(--text-sm);
  color: var(--text-secondary);
  font-weight: var(--weight-bold);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  flex-shrink: 0;
}

.info-value {
  font-size: var(--text-sm);
  color: var(--text-primary);
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-family: var(--font-mono);
}

.copy-btn {
  padding: 4px 8px;
  font-size: 11px;
  opacity: 0.8;
  transition: opacity var(--transition-fast);
}

.copy-btn:hover {
  opacity: 1;
}

.block-type {
  padding-top: var(--space-4);
  border-top: 2px solid var(--border-subtle);
  display: flex;
  justify-content: flex-end;
}

@media (max-width: 768px) {
  .block-height {
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
  }
  
  .block-number {
    font-size: var(--text-xl);
  }
  
  .info-row {
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-1);
  }
}
</style>
