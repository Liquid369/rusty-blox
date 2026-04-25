<template>
  <div class="time-range-selector">
    <button
      v-for="range in ranges"
      :key="range.value"
      :class="['range-button', { active: modelValue === range.value }]"
      @click="$emit('update:modelValue', range.value)"
    >
      {{ range.label }}
    </button>
  </div>
</template>

<script setup>
defineProps({
  modelValue: {
    type: String,
    required: true
  },
  ranges: {
    type: Array,
    default: () => [
      { value: '24h', label: '24H' },
      { value: '7d', label: '7D' },
      { value: '30d', label: '30D' },
      { value: '90d', label: '90D' },
      { value: '1y', label: '1Y' },
      { value: 'all', label: 'All' }
    ]
  }
})

defineEmits(['update:modelValue'])
</script>

<style scoped>
.time-range-selector {
  display: flex;
  gap: var(--space-2);
  flex-wrap: wrap;
}

.range-button {
  padding: var(--space-2) var(--space-4);
  background: var(--card-bg);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  color: var(--text-secondary);
  font-weight: var(--weight-medium);
  font-size: var(--text-sm);
  cursor: pointer;
  transition: all 0.2s ease;
}

.range-button:hover {
  background: rgba(255, 255, 255, 0.05);
  border-color: var(--text-accent);
  color: var(--text-primary);
}

.range-button.active {
  background: var(--text-accent);
  border-color: var(--text-accent);
  color: var(--text-dark);
  font-weight: var(--weight-bold);
}
</style>
