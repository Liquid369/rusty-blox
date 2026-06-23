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
  background: rgba(var(--rgb-purple-dark), 0.5);
  border: 1px solid var(--border-secondary);
  border-radius: var(--radius-full);
  color: var(--text-secondary);
  font-weight: var(--weight-medium);
  font-size: var(--text-sm);
  cursor: pointer;
  transition:
    background-color var(--transition-fast),
    border-color var(--transition-fast),
    color var(--transition-fast),
    box-shadow var(--transition-fast);
}

.range-button:hover {
  background: rgba(var(--rgb-purple-mid), 0.6);
  border-color: rgba(var(--rgb-purple-accent), 0.5);
  color: var(--text-primary);
}

.range-button:focus-visible {
  outline: 2px solid var(--focus-ring-color);
  outline-offset: 2px;
}

.range-button.active {
  background: linear-gradient(180deg, var(--pivx-purple-light), var(--pivx-purple-primary));
  border-color: var(--purple-accent);
  color: var(--text-primary);
  font-weight: var(--weight-bold);
  box-shadow: var(--glow-purple);
}
</style>
