<template>
  <div class="tabs">
    <div class="tabs-header">
      <button
        v-for="tab in tabs"
        :key="tab.value"
        :class="['tab-button', { active: modelValue === tab.value }]"
        @click="$emit('update:modelValue', tab.value)"
      >
        {{ tab.label }}
        <Badge v-if="tab.badge" size="sm" :variant="modelValue === tab.value ? 'accent' : 'default'">
          {{ tab.badge }}
        </Badge>
      </button>
    </div>
    <div class="tabs-content">
      <slot />
    </div>
  </div>
</template>

<script setup>
import Badge from './Badge.vue'

defineProps({
  tabs: {
    type: Array,
    required: true
  },
  modelValue: {
    type: String,
    required: true
  }
})

defineEmits(['update:modelValue'])
</script>

<style scoped>
.tabs {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.tabs-header {
  display: flex;
  gap: var(--space-2);
  border-bottom: 1px solid var(--border-secondary);
  overflow-x: auto;
}

.tab-button {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-4);
  background: none;
  border: none;
  border-bottom: 2px solid transparent;
  margin-bottom: -1px;
  color: var(--text-secondary);
  font-weight: var(--weight-medium);
  font-size: var(--text-sm);
  cursor: pointer;
  transition:
    color var(--transition-fast),
    background-color var(--transition-fast),
    border-color var(--transition-fast),
    box-shadow var(--transition-fast);
  white-space: nowrap;
  border-radius: var(--radius-sm) var(--radius-sm) 0 0;
}

.tab-button:hover {
  color: var(--text-primary);
  background: var(--bg-hover);
}

.tab-button:focus-visible {
  outline: 2px solid var(--focus-ring-color);
  outline-offset: -2px;
}

.tab-button.active {
  color: var(--text-primary);
  font-weight: var(--weight-semibold);
  border-bottom-color: var(--purple-accent);
  box-shadow: inset 0 -10px 16px -14px rgba(var(--rgb-purple-accent), 0.7);
}

.tabs-content {
  padding-top: var(--space-2);
}
</style>
