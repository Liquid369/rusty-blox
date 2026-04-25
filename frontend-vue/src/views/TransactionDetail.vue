<template>
  <AppLayout>
    <div class="tx-detail-page">
      <h1>Transaction Detail</h1>
      <p class="text-secondary">Transaction ID: {{ $route.params.txid }}</p>
      <UiCard class="mt-6">
        <p>Transaction detail view - Coming soon</p>
      </UiCard>
    </div>
  </AppLayout>
</template>

<script setup>
import AppLayout from '@/components/layout/AppLayout.vue'
import UiCard from '@/components/common/UiCard.vue'
</script>
