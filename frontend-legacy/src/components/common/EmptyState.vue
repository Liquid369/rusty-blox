<template>
  <div class="empty-state">
    <div class="empty-icon">{{ icon }}</div>
    <h3 class="empty-title">{{ title }}</h3>
    <p class="empty-message">{{ message }}</p>
    <slot name="action" />
  </div>
</template>

<script setup>
defineProps({
  icon: {
    type: String,
    default: '📭'
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
}

.empty-icon {
  font-size: 64px;
  margin-bottom: var(--space-4);
  opacity: 0.5;
}

.empty-title {
  font-size: var(--text-2xl);
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
