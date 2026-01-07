<template>
  <div
    :class="['skeleton', `skeleton-${variant}`]"
    :style="computedStyle"
  ></div>
</template>

<script setup>
import { computed } from 'vue'

const props = defineProps({
  variant: {
    type: String,
    default: 'text',
    validator: (value) => ['text', 'title', 'card', 'circle', 'button'].includes(value)
  },
  width: {
    type: String,
    default: ''
  },
  height: {
    type: String,
    default: ''
  }
})

const computedStyle = computed(() => {
  const style = {}
  if (props.width) style.width = props.width
  if (props.height) style.height = props.height
  return style
})
</script>

<style scoped>
.skeleton {
  background: linear-gradient(
    90deg,
    var(--bg-tertiary) 0%,
    var(--bg-secondary) 50%,
    var(--bg-tertiary) 100%
  );
  background-size: 200% 100%;
  animation: shimmer 1.5s ease-in-out infinite;
  border-radius: var(--radius-md);
}

.skeleton-text {
  height: 16px;
  width: 100%;
  margin-bottom: var(--space-2);
}

.skeleton-title {
  height: 28px;
  width: 60%;
  margin-bottom: var(--space-3);
}

.skeleton-card {
  height: 120px;
  width: 100%;
}

.skeleton-circle {
  height: 40px;
  width: 40px;
  border-radius: 50%;
}

.skeleton-button {
  height: 40px;
  width: 120px;
  border-radius: var(--radius-md);
}

@keyframes shimmer {
  0% {
    background-position: 200% 0;
  }
  100% {
    background-position: -200% 0;
  }
}
</style>
