<template>
  <footer class="app-footer">
    <div class="footer-container">
      <div class="footer-brand">
        <img 
          src="/PIVX-Horz-White.svg" 
          alt="PIVX Logo" 
          class="footer-logo"
          @error="(e) => e.target.style.display = 'none'"
        >
        <div class="footer-tagline">
          <p class="tagline-text">Private. Instant. Verified Transactions.</p>
          <p class="tagline-subtext">Powered by RustyBlox</p>
        </div>
      </div>

      <nav class="footer-links">
        <a href="https://pivx.org" target="_blank" rel="noopener">PIVX.org</a>
        <span class="separator">|</span>
        <a href="https://docs.pivx.org" target="_blank" rel="noopener">Docs</a>
        <span class="separator">|</span>
        <a href="https://discord.pivx.org" target="_blank" rel="noopener">Discord</a>
        <span class="separator">|</span>
        <a href="https://github.com/PIVX-Project" target="_blank" rel="noopener">GitHub</a>
      </nav>

      <div class="footer-copyright">
        © 2019-{{ currentYear }} PIVX Community
      </div>
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
  border-top: 1px solid var(--border-subtle);
  margin-top: var(--space-16);
  padding: var(--space-8) 0;
}

.footer-container {
  max-width: 1200px;
  margin: 0 auto;
  padding: 0 var(--space-6);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-6);
  text-align: center;
}

.footer-brand {
  display: flex;
  align-items: center;
  gap: var(--space-4);
}

.footer-logo {
  height: 36px;
  width: auto;
  opacity: 0.7;
}

.footer-tagline {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  text-align: left;
}

.tagline-text {
  font-size: var(--text-sm);
  color: var(--text-secondary);
  font-weight: var(--weight-bold);
  line-height: 1;
}

.tagline-subtext {
  font-size: var(--text-xs);
  color: var(--text-tertiary);
}

.footer-links {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex-wrap: wrap;
  justify-content: center;
}

.footer-links a {
  color: var(--color-primary);
  text-decoration: none;
  font-size: var(--text-sm);
  font-weight: var(--weight-bold);
  transition: color var(--transition-fast);
}

.footer-links a:hover {
  color: var(--text-accent);
}

.separator {
  color: var(--text-tertiary);
  font-size: var(--text-sm);
}

.footer-copyright {
  font-size: var(--text-xs);
  color: var(--text-tertiary);
}

@media (max-width: 768px) {
  .footer-container {
    padding: 0 var(--space-4);
  }

  .footer-brand {
    flex-direction: column;
    gap: var(--space-2);
  }

  .footer-tagline {
    text-align: center;
  }

  .footer-links {
    font-size: var(--text-xs);
  }
}
</style>
