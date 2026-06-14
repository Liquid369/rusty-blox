<template>
  <div class="stat-card" :class="{ loading: isLoading }">
    <div v-if="isLoading" class="skeleton"></div>
    <template v-else>
      <div class="stat-label">
        <span v-if="icon" class="stat-icon"><Icon :name="icon" :size="18" /></span>
        {{ label }}
      </div>
      <div class="stat-value" :class="valueClass">
        {{ value }}<span v-if="suffix" class="stat-suffix">{{ suffix }}</span>
      </div>
      <div v-if="subtitle" class="stat-subtitle">
        {{ subtitle }}
      </div>
    </template>
  </div>
</template>

<script setup>
import Icon from './Icon.vue'

defineProps({
  label: {
    type: String,
    required: true
  },
  value: {
    type: [String, Number],
    required: true
  },  suffix: {
    type: String,
    default: ''
  },  suffix: {
    type: String,
    default: ''
  },
  subtitle: {
    type: String,
    default: ''
  },
  icon: {
    type: String,
    default: ''
  },
  valueClass: {
    type: String,
    default: ''
  },
  isLoading: {
    type: Boolean,
    default: false
  }
})
</script>

<style scoped>
.stat-card {
  background: var(--glass-bg);
  border: 1px solid var(--glass-border);
  border-radius: var(--radius-lg);
  backdrop-filter: blur(var(--blur-md));
  -webkit-backdrop-filter: blur(var(--blur-md));
  box-shadow: var(--shadow-sm), var(--glass-highlight);
  padding: var(--space-6);
  transition:
    transform var(--transition-base),
    border-color var(--transition-base),
    box-shadow var(--transition-base);
  min-height: 140px;
  display: flex;
  flex-direction: column;
  justify-content: center;
  position: relative;
  overflow: hidden;
}

.stat-card::before {
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

.stat-card:hover {
  transform: translateY(-2px);
  box-shadow: var(--shadow-md), var(--glow-purple), var(--glass-highlight);
  border-color: var(--glass-border-hover);
}

.stat-card:hover::before {
  opacity: 1;
}

.stat-card.loading {
  pointer-events: none;
  opacity: 0.7;
}

.stat-label {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--text-sm);
  color: var(--text-secondary);
  font-weight: var(--weight-bold);
  text-transform: uppercase;
  letter-spacing: 0.8px;
  margin-bottom: var(--space-3);
}

.stat-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--text-accent);
  filter: drop-shadow(0 2px 4px rgba(0, 0, 0, 0.3));
}

.stat-value {
  font-size: var(--text-xl);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
  line-height: 1.2;
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
  margin-bottom: var(--space-2);
  text-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
  word-break: break-all;
  display: flex;
  align-items: baseline;
  gap: var(--space-2);
  flex-wrap: wrap;
}

.stat-suffix {
  font-size: var(--text-base);
  font-weight: var(--weight-semibold);
  color: var(--text-secondary);
  font-family: var(--font-primary);
  text-transform: uppercase;
}

.stat-value.text-success {
  color: var(--success);
  text-shadow: 0 0 20px rgba(179, 255, 120, 0.3);
}

.stat-value.text-warning {
  color: var(--warning);
  text-shadow: 0 0 20px rgba(245, 158, 11, 0.3);
}

.stat-value.text-accent {
  color: var(--text-accent);
  text-shadow: 0 0 20px rgba(179, 255, 120, 0.3);
}

.stat-subtitle {
  font-size: var(--text-xs);
  color: var(--text-tertiary);
  font-weight: var(--weight-regular);
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.skeleton {
  width: 100%;
  height: 90px;
  border-radius: var(--radius-md);
  background: linear-gradient(
    90deg,
    var(--bg-tertiary) 0%,
    var(--bg-quaternary) 50%,
    var(--bg-tertiary) 100%
  );
  background-size: 200% 100%;
  animation: shimmer 1.5s infinite;
}

@keyframes shimmer {
  0% {
    background-position: 200% 0;
  }
  100% {
    background-position: -200% 0;
  }
}

@media (max-width: 768px) {
  .stat-card {
    padding: var(--space-5);
    min-height: 120px;
  }

  .stat-value {
    font-size: var(--text-2xl);
  }
  
  .stat-icon .ui-icon {
    width: 16px;
    height: 16px;
  }
}
</style>
