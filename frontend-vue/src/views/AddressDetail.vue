<template>
  <AppLayout>
    <div class="address-detail-page">
      <h1>Address Detail</h1>
      <p class="mono text-secondary">{{ $route.params.address }}</p>
      <UiCard class="mt-6">
        <p>Address detail view - Coming soon</p>
      </UiCard>
    </div>
  </AppLayout>
</template>

<script setup>
import AppLayout from '@/components/layout/AppLayout.vue'
import UiCard from '@/components/common/UiCard.vue'
</script>
