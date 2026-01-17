<template>
  <div :class="['card', { 'card-hover': hover, 'card-clickable': clickable }]" @click="handleClick">
    <div v-if="$slots.header" class="card-header">
      <slot name="header" />
    </div>
    <div class="card-body">
      <slot />
    </div>
    <div v-if="$slots.footer" class="card-footer">
      <slot name="footer" />
    </div>
  </div>
</template>

<script setup>
const props = defineProps({
  hover: {
    type: Boolean,
    default: false
  },
  clickable: {
    type: Boolean,
    default: false
  }
})

const emit = defineEmits(['click'])

const handleClick = (event) => {
  if (props.clickable) {
    emit('click', event)
  }
}
</script>

<style scoped>
.card {
  background: var(--bg-secondary);
  border: 2px solid var(--border-primary);
  border-radius: var(--radius-md);
  padding: var(--space-4);
  transition: all var(--transition-base);
}

.card-hover:hover {
  transform: translateY(-2px);
  box-shadow: var(--shadow-lg);
}

.card-clickable {
  cursor: pointer;
}

.card-header {
  padding-bottom: var(--space-4);
  border-bottom: 1px solid var(--border-subtle);
  margin-bottom: var(--space-4);
}

.card-body {
  /* Body content */
}

.card-footer {
  padding-top: var(--space-4);
  border-top: 1px solid var(--border-subtle);
  margin-top: var(--space-4);
}
</style>
