<template>
  <footer class="app-footer">
    <div class="footer-container">
      <div class="footer-section">
        <h4>PIVX Explorer</h4>
        <p class="footer-text">Open-source blockchain explorer for PIVX</p>
      </div>

      <div class="footer-section">
        <h4>Resources</h4>
        <a href="https://pivx.org" target="_blank" rel="noopener" class="footer-link">PIVX.org</a>
        <a href="https://github.com/PIVX-Project" target="_blank" rel="noopener" class="footer-link">GitHub</a>
        <a href="https://discord.gg/pivx" target="_blank" rel="noopener" class="footer-link">Discord</a>
      </div>

      <div class="footer-section">
        <h4>Explorer</h4>
        <router-link to="/blocks" class="footer-link">Recent Blocks</router-link>
        <router-link to="/mempool" class="footer-link">Mempool</router-link>
        <router-link to="/analytics" class="footer-link">Analytics</router-link>
      </div>
    </div>

    <div class="footer-bottom">
      <p class="footer-copyright">
        © {{ currentYear }} PIVX. Open-source under MIT License.
      </p>
    </div>
  </footer>
</template>

<script setup>
import { computed } from 'vue'

const currentYear = computed(() => new Date().getFullYear())
</script>

<style scoped>
.app-footer {
  background: var(--bg-secondary);
  border-top: 2px solid var(--border-primary);
  margin-top: var(--space-16);
}

.footer-container {
  max-width: 1200px;
  margin: 0 auto;
  padding: var(--space-8) var(--space-6);
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: var(--space-8);
}

.footer-section h4 {
  font-size: var(--text-lg);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
  margin-bottom: var(--space-3);
}

.footer-text {
  color: var(--text-secondary);
  font-size: var(--text-sm);
}

.footer-link {
  display: block;
  color: var(--text-secondary);
  font-size: var(--text-sm);
  margin-bottom: var(--space-2);
  transition: color var(--transition-fast);
}

.footer-link:hover {
  color: var(--text-accent);
}

.footer-bottom {
  border-top: 1px solid var(--border-subtle);
  padding: var(--space-4) var(--space-6);
  text-align: center;
}

.footer-copyright {
  color: var(--text-tertiary);
  font-size: var(--text-xs);
}

@media (max-width: 768px) {
  .footer-container {
    grid-template-columns: 1fr;
    gap: var(--space-6);
  }
}
</style>
