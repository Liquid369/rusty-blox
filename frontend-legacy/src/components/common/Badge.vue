<template>
  <span
    :class="[
      'badge',
      `badge-${variant}`,
      `badge-${size}`
    ]"
  >
    <slot />
  </span>
</template>

<script setup>
defineProps({
  variant: {
    type: String,
    default: 'default',
    validator: (value) => ['default', 'success', 'warning', 'danger', 'info', 'accent'].includes(value)
  },
  size: {
    type: String,
    default: 'md',
    validator: (value) => ['sm', 'md', 'lg'].includes(value)
  }
})
</script>

<style scoped>
.badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-family: var(--font-primary);
  font-weight: var(--weight-bold);
  border-radius: var(--radius-sm);
  white-space: nowrap;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

/* Sizes */
.badge-sm {
  padding: 2px var(--space-2);
  font-size: 10px;
  line-height: 1.4;
}

.badge-md {
  padding: 4px var(--space-3);
  font-size: 11px;
  line-height: 1.4;
}

.badge-lg {
  padding: 6px var(--space-4);
  font-size: 12px;
  line-height: 1.4;
}

/* Default Variant */
.badge-default {
  background: var(--bg-tertiary);
  color: var(--text-secondary);
  border: 1px solid var(--border-subtle);
}

/* Success Variant */
.badge-success {
  background: rgba(34, 197, 94, 0.15);
  color: #4ade80;
  border: 1px solid rgba(34, 197, 94, 0.3);
}

/* Warning Variant */
.badge-warning {
  background: rgba(251, 191, 36, 0.15);
  color: #fbbf24;
  border: 1px solid rgba(251, 191, 36, 0.3);
}

/* Danger Variant */
.badge-danger {
  background: rgba(239, 68, 68, 0.15);
  color: #fca5a5;
  border: 1px solid rgba(239, 68, 68, 0.3);
}

/* Info Variant */
.badge-info {
  background: rgba(59, 130, 246, 0.15);
  color: #60a5fa;
  border: 1px solid rgba(59, 130, 246, 0.3);
}

/* Accent Variant */
.badge-accent {
  background: rgba(89, 252, 179, 0.15);
  color: var(--pivx-accent);
  border: 1px solid rgba(89, 252, 179, 0.3);
}
</style>
