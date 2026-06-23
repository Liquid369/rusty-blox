<template>
  <div class="empty-state">
    <div class="empty-icon"><Icon :name="icon" :size="56" /></div>
    <h3 class="empty-title">{{ title }}</h3>
    <p class="empty-message">{{ message }}</p>
    <slot name="action" />
  </div>
</template>

<script setup>
import Icon from './Icon.vue'

defineProps({
  icon: {
    type: String,
    default: 'inbox'
  },
  title: {
    type: String,
    default: 'No Data'
  },
  message: {
    type: String,
    default: 'There is no data to display at this time.'
  }
})
</script>

<style scoped>
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: var(--space-16) var(--space-6);
  text-align: center;
  min-height: 300px;
  border: 1px dashed var(--border-secondary);
  border-radius: var(--radius-lg);
  background: rgba(var(--rgb-purple-dark), 0.3);
}

.empty-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
  margin-bottom: var(--space-4);
  opacity: 0.5;
}

.empty-title {
  font-size: var(--text-xl);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
  margin-bottom: var(--space-2);
}

.empty-message {
  font-size: var(--text-base);
  color: var(--text-tertiary);
  max-width: 400px;
  margin-bottom: var(--space-6);
}
</style>
