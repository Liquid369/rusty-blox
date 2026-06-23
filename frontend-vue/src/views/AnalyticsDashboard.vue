<template>
  <AppLayout>
    <div class="analytics-dashboard">
      <h1>Analytics</h1>
      <p class="page-subtitle">Blockchain insights and visualizations</p>

      <!-- Tab Navigation -->
      <div class="tab-bar" role="tablist">
        <button
          v-for="tab in tabs"
          :key="tab.value"
          role="tab"
          :aria-selected="activeTab === tab.value"
          :class="['tab-btn', { active: activeTab === tab.value }]"
          @click="activeTab = tab.value"
        >
          {{ tab.label }}
        </button>
      </div>

      <!-- Tab Content -->
      <div class="tab-content">
        <KeepAlive>
          <component :is="activeComponent" />
        </KeepAlive>
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, computed } from 'vue'
import AppLayout from '@/components/layout/AppLayout.vue'
import SupplyAnalytics from '@/components/analytics/SupplyAnalytics.vue'
import TransactionAnalytics from '@/components/analytics/TransactionAnalytics.vue'
import StakingAnalytics from '@/components/analytics/StakingAnalytics.vue'
import NetworkHealth from '@/components/analytics/NetworkHealth.vue'
import RichList from '@/components/analytics/RichList.vue'

const tabs = [
  { value: 'supply', label: 'Supply', component: SupplyAnalytics },
  { value: 'transactions', label: 'Transactions', component: TransactionAnalytics },
  { value: 'staking', label: 'Staking', component: StakingAnalytics },
  { value: 'network', label: 'Network Health', component: NetworkHealth },
  { value: 'richlist', label: 'Rich List', component: RichList }
]

const activeTab = ref('supply')

const activeComponent = computed(() => {
  return tabs.find((t) => t.value === activeTab.value)?.component || SupplyAnalytics
})
</script>

<style scoped>
.analytics-dashboard {
  animation: fadeIn 0.3s ease;
}

.analytics-dashboard h1 {
  font-size: var(--text-4xl);
  font-weight: var(--weight-extrabold);
  margin-bottom: var(--space-2);
  color: var(--text-primary);
}

.page-subtitle {
  color: var(--text-secondary);
  margin-bottom: var(--space-8);
}

.tab-bar {
  display: flex;
  gap: var(--space-2);
  margin-bottom: var(--space-8);
  border-bottom: 2px solid var(--border-primary);
  overflow-x: auto;
}

.tab-btn {
  padding: var(--space-3) var(--space-4);
  background: none;
  border: none;
  color: var(--text-secondary);
  font-family: var(--font-primary);
  font-size: var(--text-base);
  font-weight: var(--weight-bold);
  cursor: pointer;
  border-bottom: 3px solid transparent;
  margin-bottom: -2px;
  transition: all var(--transition-fast);
  white-space: nowrap;
}

.tab-btn:hover {
  color: var(--text-primary);
}

.tab-btn.active {
  color: var(--text-accent);
  border-bottom-color: var(--border-accent);
}

.tab-content {
  animation: fadeIn 0.3s ease;
}

@keyframes fadeIn {
  from {
    opacity: 0;
    transform: translateY(10px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

@media (max-width: 768px) {
  .tab-btn {
    font-size: var(--text-sm);
    padding: var(--space-2) var(--space-3);
  }
}
</style>
