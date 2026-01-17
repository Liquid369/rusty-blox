<template>
  <AppLayout>
    <div class="page">
      <h1>XPub Detail</h1>
      <p class="placeholder">XPub detail view coming soon...</p>
      <p class="text-sm font-mono">XPub: {{ $route.params.xpub }}</p>
    </div>
  </AppLayout>
</template>

<script setup>
import AppLayout from '@/components/layout/AppLayout.vue'
</script>

<style scoped>
.page { padding: var(--space-6); }
.placeholder { color: var(--text-tertiary); font-style: italic; padding: var(--space-8); text-align: center; }
</style>
