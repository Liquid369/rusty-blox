<template>
  <AppLayout>
    <div class="not-found">
      <div class="error-code">404</div>
      <h1>Page Not Found</h1>
      <p>The page you're looking for doesn't exist.</p>
      <RouterLink to="/" class="home-link">Go to Dashboard</RouterLink>
    </div>
  </AppLayout>
</template>

<script setup>
import AppLayout from '@/components/layout/AppLayout.vue'
</script>

<style scoped>
.not-found {
  padding: var(--space-16) var(--space-6);
  text-align: center;
  min-height: 60vh;
  display: flex;
  flex-direction: column;
  justify-content: center;
  align-items: center;
}

.error-code {
  font-size: 120px;
  font-weight: var(--weight-extrabold);
  color: var(--pivx-purple-primary);
  line-height: 1;
  margin-bottom: var(--space-4);
}

.not-found h1 {
  margin-bottom: var(--space-4);
}

.not-found p {
  color: var(--text-secondary);
  margin-bottom: var(--space-6);
}

.home-link {
  display: inline-block;
  padding: var(--space-3) var(--space-6);
  background: var(--pivx-purple-primary);
  color: white;
  border-radius: var(--radius-md);
  font-weight: var(--weight-bold);
  transition: background var(--transition-fast);
}

.home-link:hover {
  background: var(--pivx-purple-dark);
}
</style>
