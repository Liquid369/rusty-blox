<template>
  <Teleport to="body">
    <Transition name="drawer">
      <div
        v-if="open"
        class="mobile-menu"
        role="dialog"
        aria-modal="true"
        aria-label="Site navigation"
        @keydown.esc.prevent="emit('close')"
      >
        <div class="mobile-menu__backdrop" @click="emit('close')" />

        <aside ref="panel" class="mobile-menu__panel" tabindex="-1">
          <div class="mobile-menu__header">
            <span class="mobile-menu__title">Menu</span>
            <button
              ref="closeBtn"
              type="button"
              class="mobile-menu__close"
              aria-label="Close navigation menu"
              @click="emit('close')"
            >
              <svg
                width="22"
                height="22"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="1.75"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <path d="M18 6 6 18 M6 6l12 12" />
              </svg>
            </button>
          </div>

          <div class="mobile-menu__search">
            <SearchBar />
          </div>

          <nav class="mobile-menu__nav" aria-label="Primary">
            <RouterLink
              v-for="link in links"
              :key="link.to"
              :to="link.to"
              class="mobile-menu__link"
              @click="emit('close')"
            >
              {{ link.label }}
            </RouterLink>
          </nav>
        </aside>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup>
import { ref, watch, nextTick, onBeforeUnmount } from 'vue'
import SearchBar from './SearchBar.vue'

const props = defineProps({
  open: {
    type: Boolean,
    default: false
  }
})

const emit = defineEmits(['close'])

// Mirror of the desktop primary nav (AppHeader .main-nav) — keep in sync.
const links = [
  { to: '/', label: 'Dashboard' },
  { to: '/blocks', label: 'Blocks' },
  { to: '/mempool', label: 'Mempool' },
  { to: '/masternodes', label: 'Masternodes' },
  { to: '/governance', label: 'Governance' },
  { to: '/analytics', label: 'Analytics' }
]

const panel = ref(null)
const closeBtn = ref(null)
let lastFocused = null

const lockScroll = (locked) => {
  document.body.style.overflow = locked ? 'hidden' : ''
}

watch(
  () => props.open,
  async (isOpen) => {
    if (isOpen) {
      lastFocused = document.activeElement
      lockScroll(true)
      await nextTick()
      closeBtn.value?.focus()
    } else {
      lockScroll(false)
      if (lastFocused && typeof lastFocused.focus === 'function') {
        lastFocused.focus()
        lastFocused = null
      }
    }
  }
)

onBeforeUnmount(() => {
  lockScroll(false)
})
</script>

<style scoped>
.mobile-menu {
  position: fixed;
  inset: 0;
  z-index: var(--z-modal);
  display: flex;
  justify-content: flex-end;
}

.mobile-menu__backdrop {
  position: absolute;
  inset: 0;
  background: rgba(var(--rgb-purple-darkest), 0.6);
  backdrop-filter: blur(var(--blur-sm));
  -webkit-backdrop-filter: blur(var(--blur-sm));
}

.mobile-menu__panel {
  position: relative;
  width: min(86vw, 340px);
  height: 100%;
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
  padding: var(--space-5);
  background: linear-gradient(
    180deg,
    rgba(var(--rgb-purple-mid), 0.96),
    rgba(var(--rgb-purple-darkest), 0.96)
  );
  border-left: 1px solid var(--glass-border);
  backdrop-filter: blur(var(--blur-lg));
  -webkit-backdrop-filter: blur(var(--blur-lg));
  box-shadow: var(--shadow-xl), var(--glass-highlight);
  overflow-y: auto;
}

.mobile-menu__panel:focus {
  outline: none;
}

.mobile-menu__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.mobile-menu__title {
  font-size: var(--text-xs);
  font-weight: var(--weight-bold);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wider);
  color: var(--text-tertiary);
}

.mobile-menu__close {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 40px;
  height: 40px;
  background: rgba(var(--rgb-purple-darkest), 0.5);
  border: 1px solid var(--glass-border);
  border-radius: var(--radius-full);
  color: var(--text-secondary);
  cursor: pointer;
  transition:
    background-color var(--transition-fast),
    border-color var(--transition-fast),
    color var(--transition-fast);
}

.mobile-menu__close:hover {
  background: rgba(var(--rgb-purple-accent), 0.3);
  color: var(--text-primary);
}

.mobile-menu__close:focus-visible {
  outline: 2px solid var(--green-accent);
  outline-offset: 2px;
  box-shadow: var(--glow-green);
}

.mobile-menu__nav {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.mobile-menu__link {
  display: block;
  padding: var(--space-3) var(--space-4);
  border-radius: var(--radius-md);
  border: 1px solid transparent;
  color: rgba(255, 255, 255, 0.82);
  text-decoration: none;
  font-weight: var(--weight-semibold);
  font-size: var(--text-base);
  letter-spacing: var(--tracking-wide);
  transition:
    background-color var(--transition-fast),
    border-color var(--transition-fast),
    color var(--transition-fast),
    box-shadow var(--transition-fast);
}

.mobile-menu__link:hover {
  background: rgba(var(--rgb-purple-darkest), 0.45);
  color: var(--text-primary);
}

.mobile-menu__link:focus-visible {
  outline: 2px solid var(--green-accent);
  outline-offset: 2px;
  box-shadow: var(--glow-green);
}

.mobile-menu__link.router-link-active {
  background: rgba(var(--rgb-purple-darkest), 0.6);
  border-color: rgba(var(--rgb-green-accent), 0.45);
  color: var(--green-accent);
  box-shadow: var(--glow-green);
}

/* Drawer transition (backdrop fades, panel slides) */
.drawer-enter-active,
.drawer-leave-active {
  transition: opacity var(--duration-base) var(--ease-out);
}

.drawer-enter-active .mobile-menu__panel,
.drawer-leave-active .mobile-menu__panel {
  transition: transform var(--duration-base) var(--ease-out);
}

.drawer-enter-from,
.drawer-leave-to {
  opacity: 0;
}

.drawer-enter-from .mobile-menu__panel,
.drawer-leave-to .mobile-menu__panel {
  transform: translateX(100%);
}

@media (prefers-reduced-motion: reduce) {
  .drawer-enter-active,
  .drawer-leave-active,
  .drawer-enter-active .mobile-menu__panel,
  .drawer-leave-active .mobile-menu__panel {
    transition: opacity var(--duration-fast) linear;
  }

  .drawer-enter-from .mobile-menu__panel,
  .drawer-leave-to .mobile-menu__panel {
    transform: none;
  }
}
</style>
