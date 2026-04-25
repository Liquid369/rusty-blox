<template>
  <div class="stat-card">
    <div class="stat-label">{{ label }}</div>
    <div v-if="loading" class="skeleton stat-value-skeleton"></div>
    <div v-else class="stat-value">{{ formattedValue }}</div>
    <div v-if="subtitle" class="stat-subtitle">{{ subtitle }}</div>
  </div>
</template>

<script setup>
import { computed } from 'vue'

const props = defineProps({
  label: {
    type: String,
    required: true
  },
  value: {
    type: [String, Number],
    default: ''
  },
  subtitle: {
    type: String,
    default: ''
  },
  loading: {
    type: Boolean,
    default: false
  },
  format: {
    type: String,
    default: 'text'
  }
})

const formattedValue = computed(() => {
  if (!props.value) return '—'
  
  if (props.format === 'number') {
    return Number(props.value).toLocaleString()
  }
  
  if (props.format === 'percentage') {
    return `${props.value}%`
  }
  
  return props.value
})
</script>

<style scoped>
.stat-card {
  background: var(--bg-secondary);
  border: 2px solid var(--border-primary);
  border-radius: var(--radius-md);
  padding: var(--space-6);
  text-align: center;
}

.stat-label {
  font-size: var(--text-sm);
  font-weight: var(--weight-bold);
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.05em;
  margin-bottom: var(--space-2);
}

.stat-value {
  font-size: var(--text-3xl);
  font-weight: var(--weight-extrabold);
  color: var(--text-primary);
  margin-bottom: var(--space-1);
}

.stat-value-skeleton {
  height: 48px;
  margin: var(--space-2) auto;
  width: 80%;
}

.stat-subtitle {
  font-size: var(--text-xs);
  color: var(--text-tertiary);
}
</style>
