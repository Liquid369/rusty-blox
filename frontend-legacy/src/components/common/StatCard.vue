<template>
  <div class="stat-card" :class="{ loading: isLoading }">
    <div v-if="isLoading" class="skeleton"></div>
    <template v-else>
      <div class="stat-label">
        <span v-if="icon" class="stat-icon">{{ icon }}</span>
        {{ label }}
      </div>
      <div class="stat-value" :class="valueClass">
        {{ value }}
      </div>
      <div v-if="subtitle" class="stat-subtitle">
        {{ subtitle }}
      </div>
    </template>
  </div>
</template>

<script setup>
defineProps({
  label: {
    type: String,
    required: true
  },
  value: {
    type: [String, Number],
    required: true
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
  background: linear-gradient(135deg, var(--bg-secondary) 0%, var(--bg-quaternary) 100%);
  border: 2px solid var(--border-secondary);
  border-radius: var(--radius-lg);
  padding: var(--space-6);
  transition: all var(--transition-base);
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
  height: 3px;
  background: linear-gradient(90deg, var(--pivx-purple-primary) 0%, var(--pivx-accent) 100%);
  opacity: 0;
  transition: opacity var(--transition-base);
}

.stat-card:hover {
  transform: translateY(-4px);
  box-shadow: 
    0 12px 24px rgba(0, 0, 0, 0.5),
    0 0 30px rgba(89, 252, 179, 0.15);
  border-color: var(--border-accent);
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
  font-size: var(--text-xl);
  filter: drop-shadow(0 2px 4px rgba(0, 0, 0, 0.3));
}

.stat-value {
  font-size: var(--text-3xl);
  font-weight: var(--weight-extrabold);
  color: var(--text-primary);
  line-height: 1.1;
  font-family: var(--font-mono);
  margin-bottom: var(--space-2);
  text-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
}

.stat-value.text-success {
  color: var(--success);
  text-shadow: 0 0 20px rgba(89, 252, 179, 0.3);
}

.stat-value.text-warning {
  color: var(--warning);
  text-shadow: 0 0 20px rgba(245, 158, 11, 0.3);
}

.stat-value.text-accent {
  color: var(--text-accent);
  text-shadow: 0 0 20px rgba(89, 252, 179, 0.3);
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
  
  .stat-icon {
    font-size: var(--text-lg);
  }
}
</style>
