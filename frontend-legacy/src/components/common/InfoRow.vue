<template>
  <div class="info-row" :class="{ 'info-row-vertical': vertical }">
    <div class="info-label">
      <span v-if="icon" class="info-icon"><Icon :name="icon" :size="14" /></span>
      {{ label }}
    </div>
    <div class="info-value">
      <slot />
    </div>
  </div>
</template>

<script setup>
import Icon from './Icon.vue'

defineProps({
  label: {
    type: String,
    required: true
  },
  icon: {
    type: String,
    default: ''
  },
  vertical: {
    type: Boolean,
    default: false
  }
})
</script>

<style scoped>
.info-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-3) 0;
  border-bottom: 1px solid var(--border-subtle);
}

.info-row:last-child {
  border-bottom: none;
}

.info-row-vertical {
  flex-direction: column;
  align-items: flex-start;
  gap: var(--space-2);
}

.info-label {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--text-xs);
  font-weight: var(--weight-semibold);
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
  flex-shrink: 0;
}

.info-icon {
  display: inline-flex;
  align-items: center;
  color: var(--text-accent);
}

.info-value {
  font-size: var(--text-base);
  color: var(--text-primary);
  word-break: break-word;
  text-align: right;
}

.info-row-vertical .info-value {
  text-align: left;
  width: 100%;
}

@media (max-width: 768px) {
  .info-row {
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
  }

  .info-value {
    text-align: left;
  }
}
</style>
