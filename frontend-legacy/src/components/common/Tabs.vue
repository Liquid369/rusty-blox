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
        <Badge v-if="tab.badge" size="sm" :variant="modelValue === tab.value ? 'primary' : 'secondary'">
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
  border-bottom: 2px solid var(--border-color);
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
  margin-bottom: -2px;
  color: var(--text-secondary);
  font-weight: var(--weight-medium);
  cursor: pointer;
  transition: all 0.2s ease;
  white-space: nowrap;
}

.tab-button:hover {
  color: var(--text-primary);
  background: rgba(255, 255, 255, 0.05);
}

.tab-button.active {
  color: var(--text-accent);
  border-bottom-color: var(--text-accent);
}

.tabs-content {
  padding-top: var(--space-2);
}
</style>
