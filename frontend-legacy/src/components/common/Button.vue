<template>
  <button
    :class="[
      'btn',
      `btn-${variant}`,
      `btn-${size}`,
      { 'btn-loading': loading, 'btn-disabled': disabled || loading }
    ]"
    :disabled="disabled || loading"
    @click="handleClick"
  >
    <span v-if="loading" class="btn-spinner"></span>
    <slot v-else />
  </button>
</template>

<script setup>
const emit = defineEmits(['click'])

const props = defineProps({
  variant: {
    type: String,
    default: 'primary',
    validator: (value) => ['primary', 'secondary', 'danger', 'ghost', 'accent'].includes(value)
  },
  size: {
    type: String,
    default: 'md',
    validator: (value) => ['sm', 'md', 'lg'].includes(value)
  },
  loading: {
    type: Boolean,
    default: false
  },
  disabled: {
    type: Boolean,
    default: false
  }
})

const handleClick = (event) => {
  if (!props.loading && !props.disabled) {
    emit('click', event)
  }
}
</script>

<style scoped>
.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-family: var(--font-primary);
  font-weight: var(--weight-bold);
  border-radius: var(--radius-md);
  border: 2px solid transparent;
  cursor: pointer;
  transition: all var(--transition-fast);
  white-space: nowrap;
  text-decoration: none;
  position: relative;
}

.btn:focus-visible {
  outline: 2px solid var(--border-accent);
  outline-offset: 2px;
}

/* Sizes */
.btn-sm {
  padding: var(--space-2) var(--space-3);
  font-size: var(--text-sm);
  min-height: 32px;
}

.btn-md {
  padding: var(--space-3) var(--space-4);
  font-size: var(--text-base);
  min-height: 40px;
}

.btn-lg {
  padding: var(--space-4) var(--space-6);
  font-size: var(--text-lg);
  min-height: 48px;
}

/* Primary Variant */
.btn-primary {
  background: var(--pivx-purple-primary);
  color: white;
  border-color: var(--pivx-purple-primary);
}

.btn-primary:hover:not(.btn-disabled) {
  background: var(--pivx-purple-dark);
  border-color: var(--pivx-purple-dark);
  transform: translateY(-1px);
  box-shadow: var(--shadow-md);
}

.btn-primary:active:not(.btn-disabled) {
  transform: translateY(0);
}

/* Secondary Variant */
.btn-secondary {
  background: var(--bg-secondary);
  color: var(--text-primary);
  border-color: var(--border-primary);
}

.btn-secondary:hover:not(.btn-disabled) {
  background: var(--bg-tertiary);
  border-color: var(--border-accent);
  transform: translateY(-1px);
}

/* Danger Variant */
.btn-danger {
  background: var(--danger);
  color: white;
  border-color: var(--danger);
}

.btn-danger:hover:not(.btn-disabled) {
  background: #dc2626;
  border-color: #dc2626;
  transform: translateY(-1px);
  box-shadow: var(--shadow-md);
}

/* Ghost Variant */
.btn-ghost {
  background: transparent;
  color: var(--text-secondary);
  border-color: transparent;
}

.btn-ghost:hover:not(.btn-disabled) {
  background: var(--bg-secondary);
  color: var(--text-primary);
}

/* Accent Variant */
.btn-accent {
  background: var(--pivx-accent);
  color: var(--pivx-purple-primary);
  border-color: var(--pivx-accent);
}

.btn-accent:hover:not(.btn-disabled) {
  background: var(--pivx-accent-dark);
  border-color: var(--pivx-accent-dark);
  transform: translateY(-1px);
  box-shadow: var(--shadow-glow);
}

/* Disabled State */
.btn-disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* Loading State */
.btn-loading {
  cursor: wait;
  pointer-events: none;
}

.btn-spinner {
  display: inline-block;
  width: 16px;
  height: 16px;
  border: 2px solid currentColor;
  border-right-color: transparent;
  border-radius: 50%;
  animation: spin 0.6s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

@media (max-width: 768px) {
  .btn-lg {
    padding: var(--space-3) var(--space-5);
    font-size: var(--text-base);
    min-height: 44px;
  }
}
</style>
