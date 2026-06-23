<template>
  <div class="time-range-selector">
    <button
      v-for="range in ranges"
      :key="range.value"
      :class="['range-btn', { active: modelValue === range.value }]"
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
    default: '30d'
  },
  ranges: {
    type: Array,
    default: () => [
      { value: '7d', label: '7D' },
      { value: '30d', label: '30D' },
      { value: '90d', label: '90D' },
      { value: '1y', label: '1Y' }
    ]
  }
})

defineEmits(['update:modelValue'])
</script>

<style scoped>
.time-range-selector {
  display: inline-flex;
  gap: var(--space-1);
  padding: var(--space-1);
  background: var(--bg-secondary);
  border: 2px solid var(--border-primary);
  border-radius: var(--radius-sm);
}

.range-btn {
  padding: var(--space-2) var(--space-4);
  background: transparent;
  border: none;
  border-radius: var(--radius-sm);
  color: var(--text-secondary);
  font-family: var(--font-primary);
  font-size: var(--text-sm);
  font-weight: var(--weight-bold);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.range-btn:hover {
  color: var(--text-primary);
  background: var(--bg-tertiary);
}

.range-btn.active {
  background: var(--pivx-purple-primary);
  color: white;
}
</style>
