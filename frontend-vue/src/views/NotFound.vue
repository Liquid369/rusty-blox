<template>
  <AppLayout>
    <div class="not-found">
      <h1>404</h1>
      <p>Page not found</p>
      <UiButton @click="$router.push('/')">Go Home</UiButton>
    </div>
  </AppLayout>
</template>

<script setup>
import AppLayout from '@/components/layout/AppLayout.vue'
import UiButton from '@/components/common/UiButton.vue'
</script>

<style scoped>
.not-found {
  text-align: center;
  padding: var(--space-16);
}

.not-found h1 {
  font-size: var(--text-4xl);
  color: var(--text-accent);
  margin-bottom: var(--space-4);
}
</style>
