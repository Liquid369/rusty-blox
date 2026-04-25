<template>
  <button
    class="copy-button"
    :class="{ 'copy-button-copied': copied }"
    @click="handleCopy"
    :title="copied ? 'Copied!' : 'Click to copy'"
  >
    <span v-if="copied" class="copy-icon">✓</span>
    <span v-else class="copy-icon">📋</span>
    <span v-if="showText" class="copy-text">
      {{ copied ? 'Copied!' : 'Copy' }}
    </span>
  </button>
</template>

<script setup>
import { ref } from 'vue'

const props = defineProps({
  text: {
    type: String,
    required: true
  },
  showText: {
    type: Boolean,
    default: false
  }
})

const emit = defineEmits(['copied'])

const copied = ref(false)
let timeoutId = null

const handleCopy = async () => {
  try {
    await navigator.clipboard.writeText(props.text)
    copied.value = true
    emit('copied', props.text)
    
    // Reset after 2 seconds
    if (timeoutId) clearTimeout(timeoutId)
    timeoutId = setTimeout(() => {
      copied.value = false
    }, 2000)
  } catch (error) {
    console.error('Failed to copy:', error)
  }
}
</script>

<style scoped>
.copy-button {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  background: var(--bg-tertiary);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
  color: var(--text-secondary);
  font-size: var(--text-sm);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.copy-button:hover {
  background: var(--bg-secondary);
  border-color: var(--border-primary);
  color: var(--text-primary);
}

.copy-button:active {
  transform: scale(0.95);
}

.copy-button-copied {
  background: rgba(34, 197, 94, 0.15);
  border-color: rgba(34, 197, 94, 0.3);
  color: #4ade80;
}

.copy-icon {
  font-size: 14px;
  line-height: 1;
}

.copy-text {
  font-weight: var(--weight-bold);
  text-transform: uppercase;
  font-size: 11px;
  letter-spacing: 0.5px;
}
</style>
