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
  gap: var(--space-2);
  font-family: var(--font-primary);
  font-weight: var(--weight-bold);
  border-radius: var(--radius-md);
  border: 1px solid transparent;
  cursor: pointer;
  transition:
    background-color var(--transition-fast),
    border-color var(--transition-fast),
    color var(--transition-fast),
    box-shadow var(--transition-fast),
    transform var(--transition-fast);
  white-space: nowrap;
  text-decoration: none;
  position: relative;
}

.btn:focus-visible {
  outline: 2px solid var(--focus-ring-color);
  outline-offset: 2px;
  box-shadow: var(--focus-ring-glow);
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
  background: linear-gradient(180deg, var(--pivx-purple-light), var(--pivx-purple-primary));
  color: white;
  border-color: rgba(var(--rgb-purple-accent), 0.45);
  box-shadow: var(--shadow-xs), var(--glass-highlight);
}

.btn-primary:hover:not(.btn-disabled) {
  border-color: var(--purple-accent);
  transform: translateY(-1px);
  box-shadow: var(--shadow-sm), var(--glow-purple);
}

.btn-primary:active:not(.btn-disabled) {
  transform: translateY(0);
  box-shadow: var(--shadow-xs);
}

/* Secondary Variant */
.btn-secondary {
  background: rgba(var(--rgb-purple-mid), 0.35);
  color: var(--text-primary);
  border-color: var(--border-primary);
}

.btn-secondary:hover:not(.btn-disabled) {
  background: rgba(var(--rgb-purple-mid), 0.6);
  border-color: var(--border-accent);
  transform: translateY(-1px);
  box-shadow: var(--shadow-sm);
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
  background: var(--bg-hover);
  border-color: rgba(var(--rgb-purple-accent), 0.35);
  color: var(--text-primary);
}

/* Accent Variant */
.btn-accent {
  background: linear-gradient(180deg, var(--pivx-accent), var(--pivx-accent-dark));
  color: var(--text-dark);
  border-color: rgba(var(--rgb-green-accent), 0.6);
}

.btn-accent:hover:not(.btn-disabled) {
  transform: translateY(-1px);
  box-shadow: var(--shadow-sm), var(--glow-green-strong);
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

/* Touch devices: raise small/medium buttons to the 44px WCAG target-size minimum
   (covers Pagination, which renders size="sm"). Gated to coarse pointers so mouse
   users keep the denser desktop sizing. */
@media (pointer: coarse) {
  .btn-sm,
  .btn-md {
    min-height: 44px;
  }
}
</style>
