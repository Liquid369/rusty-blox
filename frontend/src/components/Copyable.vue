<script setup>
// Click-to-copy for long hex values. Shows a (usually truncated) label via the
// default slot but copies the FULL `value`, so shielded commitments/nullifiers,
// hashes, etc. stay inspectable without blowing out the layout.
import { ref } from 'vue'

const props = defineProps({
  value: { type: String, default: '' }
})
const copied = ref(false)

async function copy() {
  if (!props.value) return
  try {
    await navigator.clipboard.writeText(props.value)
  } catch {
    // Fallback for non-secure contexts (clipboard API needs https/localhost).
    const ta = document.createElement('textarea')
    ta.value = props.value
    ta.style.position = 'fixed'
    ta.style.opacity = '0'
    document.body.appendChild(ta)
    ta.select()
    try { document.execCommand('copy') } catch { /* give up silently */ }
    document.body.removeChild(ta)
  }
  copied.value = true
  setTimeout(() => { copied.value = false }, 1200)
}
</script>

<template>
  <button
    type="button"
    class="copyable mono"
    :title="copied ? 'Copied!' : `Copy: ${value}`"
    :aria-label="`Copy ${value}`"
    @click="copy"
  >
    <span class="copyable-txt"><slot>{{ value }}</slot></span>
    <span class="copyable-ic" :class="{ ok: copied }" aria-hidden="true">{{ copied ? '✓' : '⧉' }}</span>
  </button>
</template>

<style scoped>
.copyable {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  max-width: 100%;
  padding: 0;
  border: 0;
  background: none;
  color: inherit;
  font: inherit;
  cursor: pointer;
}
.copyable-txt { overflow: hidden; text-overflow: ellipsis; }
.copyable-ic {
  flex: none;
  font-size: 0.9em;
  opacity: 0.45;
  transition: opacity 120ms ease, color 120ms ease;
}
.copyable:hover .copyable-ic { opacity: 1; }
.copyable-ic.ok { color: #46e6d0; opacity: 1; }
.copyable:focus-visible {
  outline: 2px solid #46e6d0;
  outline-offset: 2px;
  border-radius: 3px;
}
</style>
