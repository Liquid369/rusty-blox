<template>
  <AppLayout>
    <div class="analytics-dashboard">
      <!-- Header -->
      <div class="page-header">
        <div>
          <h1 class="page-title">Analytics Dashboard</h1>
          <p class="page-subtitle">Comprehensive blockchain insights and visualizations</p>
        </div>
      </div>

      <!-- Navigation Tabs -->
      <Tabs v-model="activeTab" :tabs="tabs">
        <!-- Supply Analytics -->
        <div v-show="activeTab === 'supply'" class="tab-content">
          <SupplyAnalytics />
        </div>

        <!-- Transaction Analytics -->
        <div v-show="activeTab === 'transactions'" class="tab-content">
          <TransactionAnalytics />
        </div>

        <!-- Staking Analytics -->
        <div v-show="activeTab === 'staking'" class="tab-content">
          <StakingAnalytics />
        </div>

        <!-- Network Health -->
        <div v-show="activeTab === 'network'" class="tab-content">
          <NetworkHealth />
        </div>

        <!-- Rich List -->
        <div v-show="activeTab === 'richlist'" class="tab-content">
          <RichList />
        </div>
      </Tabs>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref } from 'vue'
import AppLayout from '@/components/layout/AppLayout.vue'
import Tabs from '@/components/common/Tabs.vue'
import SupplyAnalytics from '@/components/analytics/SupplyAnalytics.vue'
import TransactionAnalytics from '@/components/analytics/TransactionAnalytics.vue'
import StakingAnalytics from '@/components/analytics/StakingAnalytics.vue'
import NetworkHealth from '@/components/analytics/NetworkHealth.vue'
import RichList from '@/components/analytics/RichList.vue'

const activeTab = ref('supply')

const tabs = [
  { value: 'supply', label: 'Supply' },
  { value: 'transactions', label: 'Transactions' },
  { value: 'staking', label: 'Staking' },
  { value: 'network', label: 'Network Health' },
  { value: 'richlist', label: 'Rich List' }
]
</script>

<style scoped>
.analytics-dashboard {
  padding: var(--space-6) 0;
}

.page-header {
  margin-bottom: var(--space-8);
}

.page-title {
  margin-bottom: var(--space-2);
}

.page-subtitle {
  color: var(--text-secondary);
}

.tab-content {
  display: grid;
  gap: var(--space-6);
  animation: fadeIn 0.3s ease;
}

@keyframes fadeIn {
  from { opacity: 0; transform: translateY(10px); }
  to { opacity: 1; transform: translateY(0); }
}
</style>
