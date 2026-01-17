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
      →
    </RouterLink>
  </div>
</template>

<script setup>
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
  background: var(--bg-tertiary);
  padding: var(--space-2) var(--space-3);
  border-radius: var(--radius-sm);
  border: 1px solid var(--border-subtle);
}

.hash-text {
  color: var(--text-primary);
  font-size: var(--text-sm);
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
