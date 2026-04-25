<template>
  <button 
    :class="['btn', variantClass, { 'btn-loading': loading }]"
    :disabled="disabled || loading"
    @click="$emit('click', $event)"
  >
    <span v-if="loading" class="loading-spinner"></span>
    <slot v-else />
  </button>
</template>

<script setup>
import { computed } from 'vue'

const props = defineProps({
  variant: {
    type: String,
    default: 'primary',
    validator: (value) => ['primary', 'secondary', 'danger'].includes(value)
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

defineEmits(['click'])

const variantClass = computed(() => `btn-${props.variant}`)
</script>

<style scoped>
.btn {
  position: relative;
}

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn-loading {
  pointer-events: none;
}

.btn-danger {
  background: var(--danger);
  color: white;
}

.btn-danger:hover {
  background: #dc2626;
}
</style>
