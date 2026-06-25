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
  gap: var(--space-1);
  font-family: var(--font-primary);
  font-weight: var(--weight-bold);
  font-variant-numeric: tabular-nums;
  border-radius: var(--radius-full);
  white-space: nowrap;
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
  transition: border-color var(--transition-fast);
}

/* Sizes */
.badge-sm {
  padding: 2px var(--space-2);
  font-size: 0.625rem;
  line-height: 1.4;
}

.badge-md {
  padding: var(--space-1) var(--space-3);
  font-size: var(--text-2xs);
  line-height: 1.4;
}

.badge-lg {
  padding: var(--space-2) var(--space-4);
  font-size: var(--text-xs);
  line-height: 1.4;
}

/* Default Variant */
.badge-default {
  background: rgba(var(--rgb-purple-mid), 0.4);
  color: var(--text-secondary);
  border: 1px solid var(--border-secondary);
}

/* Success Variant - brand green */
.badge-success {
  background: rgba(var(--rgb-green-accent), 0.12);
  color: var(--green-accent);
  border: 1px solid rgba(var(--rgb-green-accent), 0.35);
}

/* Warning Variant */
.badge-warning {
  background: rgba(246, 255, 120, 0.1);
  color: var(--warning);
  border: 1px solid rgba(246, 255, 120, 0.3);
}

/* Danger Variant */
.badge-danger {
  background: rgba(239, 68, 68, 0.15);
  color: var(--danger-text);
  border: 1px solid rgba(239, 68, 68, 0.35);
}

/* Info Variant - brand purple */
.badge-info {
  background: rgba(var(--rgb-purple-accent), 0.14);
  color: var(--info); /* now AA-correct at the token (was a hardcoded #CD97F7 workaround) */
  border: 1px solid rgba(var(--rgb-purple-accent), 0.4);
}

/* Accent Variant */
.badge-accent {
  background: rgba(var(--rgb-green-accent), 0.12);
  color: var(--pivx-accent);
  border: 1px solid rgba(var(--rgb-green-accent), 0.35);
}
</style>
