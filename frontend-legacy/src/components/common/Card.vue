<template>
  <div
    :class="[
      'card',
      { 'card-hover': hover, 'card-clickable': clickable }
    ]"
    @click="handleClick"
  >
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
const emit = defineEmits(['click'])

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

const handleClick = (event) => {
  if (props.clickable) {
    emit('click', event)
  }
}
</script>

<style scoped>
.card {
  background: linear-gradient(135deg, var(--bg-secondary) 0%, var(--bg-quaternary) 100%);
  border: 2px solid var(--border-secondary);
  border-radius: var(--radius-lg);
  overflow: hidden;
  transition: all var(--transition-base);
  position: relative;
}

.card::before {
  content: '';
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 3px;
  background: linear-gradient(90deg, var(--pivx-purple-primary) 0%, var(--pivx-accent) 100%);
  opacity: 0;
  transition: opacity var(--transition-base);
}

.card:hover::before {
  opacity: 1;
}

.card-hover:hover {
  border-color: var(--border-accent);
  box-shadow: 
    0 8px 16px rgba(0, 0, 0, 0.4),
    0 0 20px rgba(89, 252, 179, 0.15);
  transform: translateY(-2px);
}

.card-clickable {
  cursor: pointer;
}

.card-clickable:hover {
  transform: translateY(-4px);
  box-shadow: 
    0 12px 24px rgba(0, 0, 0, 0.5),
    0 0 30px rgba(89, 252, 179, 0.2);
  border-color: var(--border-accent);
}

.card-clickable:active {
  transform: translateY(-2px);
}

.card-header {
  padding: var(--space-5) var(--space-6);
  border-bottom: 1px solid var(--border-subtle);
  font-weight: var(--weight-bold);
  font-size: var(--text-lg);
  background: rgba(102, 45, 145, 0.1);
  position: relative;
}

.card-body {
  padding: var(--space-6);
}

.card-footer {
  padding: var(--space-4) var(--space-6);
  border-top: 1px solid var(--border-subtle);
  background: rgba(42, 27, 66, 0.5);
  font-size: var(--text-sm);
  color: var(--text-secondary);
}

@media (max-width: 768px) {
  .card-header,
  .card-body {
    padding: var(--space-4);
  }
  
  .card-footer {
    padding: var(--space-3) var(--space-4);
  }
}
</style>
