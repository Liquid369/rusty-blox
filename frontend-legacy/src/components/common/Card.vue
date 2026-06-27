<template>
  <div
    :class="[
      'card',
      `card--${variant}`,
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
  },
  // 'glass' (default, hero/elevated) | 'data' (flat opaque surface for dense
  // tables/grids — sharper to scan, cheaper to render than glass-on-glass)
  variant: {
    type: String,
    default: 'glass'
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
  background: var(--glass-bg);
  border: 1px solid var(--glass-border);
  border-radius: var(--radius-lg);
  backdrop-filter: blur(var(--blur-md));
  -webkit-backdrop-filter: blur(var(--blur-md));
  box-shadow: var(--shadow-sm), var(--glass-highlight);
  overflow: hidden;
  transition:
    transform var(--transition-base),
    border-color var(--transition-base),
    box-shadow var(--transition-base);
  position: relative;
}

.card::before {
  content: '';
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 1px;
  background: linear-gradient(90deg, transparent, rgba(var(--rgb-purple-accent), 0.6), transparent);
  opacity: 0;
  transition: opacity var(--transition-base);
  pointer-events: none;
}

.card:hover::before {
  opacity: 1;
}

.card-hover:hover {
  border-color: var(--glass-border-hover);
  box-shadow: var(--shadow-md), var(--glow-purple), var(--glass-highlight);
  transform: translateY(-2px);
}

.card-clickable {
  cursor: pointer;
}

.card-clickable:hover {
  transform: translateY(-3px);
  box-shadow: var(--shadow-lg), var(--glow-purple), var(--glass-highlight);
  border-color: var(--glass-border-hover);
}

.card-clickable:active {
  transform: translateY(-1px);
}

.card-clickable:focus-visible {
  outline: 2px solid var(--focus-ring-color);
  outline-offset: 2px;
}

/* Data variant: flat opaque surface for dense tables/grids. Reserves the glass
   blur for hero/elevated cards and avoids muddy glass-on-glass nesting. */
.card.card--data {
  background: var(--surface-data);
  backdrop-filter: none;
  -webkit-backdrop-filter: none;
  border-color: var(--border-subtle);
  box-shadow: var(--shadow-xs);
}

.card.card--data::before {
  display: none; /* no luminous hero top-edge on data surfaces */
}

.card-header {
  padding: var(--space-5) var(--space-6);
  border-bottom: 1px solid var(--glass-border);
  font-weight: var(--weight-bold);
  font-size: var(--text-lg);
  background: rgba(var(--rgb-purple-main), 0.16);
  position: relative;
}

.card.card--data .card-header {
  background: rgba(var(--rgb-purple-main), 0.10);
  border-bottom-color: var(--border-subtle);
}

.card-body {
  padding: var(--space-6);
}

.card-footer {
  padding: var(--space-4) var(--space-6);
  border-top: 1px solid var(--border-subtle);
  background: rgba(var(--rgb-purple-darkest), 0.4);
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
