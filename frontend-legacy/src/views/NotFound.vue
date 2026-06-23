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
  color: var(--purple-accent);
  background: linear-gradient(135deg, var(--purple-accent), var(--pivx-purple-primary));
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
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
  background: linear-gradient(180deg, var(--pivx-purple-light), var(--pivx-purple-primary));
  border: 1px solid rgba(var(--rgb-purple-accent), 0.45);
  color: white;
  border-radius: var(--radius-md);
  font-weight: var(--weight-bold);
  transition: box-shadow var(--transition-fast), transform var(--transition-fast);
}

.home-link:hover {
  color: white;
  transform: translateY(-1px);
  box-shadow: var(--shadow-sm), var(--glow-purple);
}
</style>
