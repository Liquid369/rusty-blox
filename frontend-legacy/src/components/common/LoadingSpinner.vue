<template>
  <div :class="['spinner-container', { 'spinner-centered': centered }]">
    <div :class="['spinner', `spinner-${size}`]"></div>
    <p v-if="text" class="spinner-text">{{ text }}</p>
  </div>
</template>

<script setup>
defineProps({
  size: {
    type: String,
    default: 'md',
    validator: (value) => ['sm', 'md', 'lg', 'xl'].includes(value)
  },
  text: {
    type: String,
    default: ''
  },
  centered: {
    type: Boolean,
    default: true
  }
})
</script>

<style scoped>
.spinner-container {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-3);
}

.spinner-centered {
  justify-content: center;
  min-height: 200px;
}

.spinner {
  border-radius: 50%;
  border: 3px solid var(--border-subtle);
  border-top-color: var(--pivx-purple-primary);
  animation: spin 0.8s linear infinite;
}

.spinner-sm {
  width: 20px;
  height: 20px;
  border-width: 2px;
}

.spinner-md {
  width: 40px;
  height: 40px;
  border-width: 3px;
}

.spinner-lg {
  width: 60px;
  height: 60px;
  border-width: 4px;
}

.spinner-xl {
  width: 80px;
  height: 80px;
  border-width: 5px;
}

.spinner-text {
  color: var(--text-secondary);
  font-size: var(--text-sm);
  margin: 0;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
