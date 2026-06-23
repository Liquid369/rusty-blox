<template>
  <div class="hash-display">
    <span v-if="truncate" class="hash-text font-mono">
      {{ truncateHash(hash, startLength, endLength) }}
    </span>
    <span v-else class="hash-text font-mono">{{ hash }}</span>
    
    <CopyButton 
      v-if="showCopy" 
      :text="hash" 
      class="hash-copy"
    />
    
    <RouterLink 
      v-if="linkTo" 
      :to="linkTo" 
      class="hash-link"
      title="View details"
    >
      <Icon name="arrow-right" :size="14" />
    </RouterLink>
  </div>
</template>

<script setup>
import Icon from './Icon.vue'
import { truncateHash } from '@/utils/formatters'
import CopyButton from './CopyButton.vue'

defineProps({
  hash: {
    type: String,
    required: true
  },
  truncate: {
    type: Boolean,
    default: true
  },
  startLength: {
    type: Number,
    default: 8
  },
  endLength: {
    type: Number,
    default: 8
  },
  showCopy: {
    type: Boolean,
    default: true
  },
  linkTo: {
    type: String,
    default: ''
  }
})
</script>

<style scoped>
.hash-display {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  background: rgba(var(--rgb-purple-darkest), 0.5);
  padding: var(--space-2) var(--space-3);
  border-radius: var(--radius-sm);
  border: 1px solid var(--border-secondary);
  transition: border-color var(--transition-fast);
}

.hash-display:hover {
  border-color: rgba(var(--rgb-purple-accent), 0.45);
}

.hash-text {
  color: var(--text-primary);
  font-size: var(--text-sm);
  font-variant-numeric: tabular-nums;
  user-select: all;
}

.hash-copy {
  padding: 2px 4px;
  font-size: 10px;
}

.hash-link {
  color: var(--text-accent);
  font-weight: var(--weight-bold);
  font-size: var(--text-lg);
  transition: color var(--transition-fast);
  text-decoration: none;
  padding: 0 var(--space-1);
}

.hash-link:hover {
  color: var(--pivx-accent-dark);
}

@media (max-width: 768px) {
  .hash-display {
    flex-wrap: wrap;
  }
}
</style>
