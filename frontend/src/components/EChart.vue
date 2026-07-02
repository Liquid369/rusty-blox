<script setup>
/* =====================================================================
   Minimal echarts wrapper. Pass an `option` object; it inits, updates on
   change, and resizes with the container. NEUTRAL: no baked theme beyond
   transparent background — prototypes pass brand colors via the option
   (read the --chart-* tokens from tokens.css). Proves echarts renders.
   ===================================================================== */
import { ref, onMounted, onBeforeUnmount, watch } from 'vue'
import { echarts } from '../lib/chart.js'

const props = defineProps({
  option: { type: Object, required: true },
  height: { type: String, default: '300px' },
  // Canvas charts are opaque to assistive tech; expose role="img" + a label.
  ariaLabel: { type: String, default: 'Data visualization chart' }
})

const el = ref(null)
let chart = null

// Respect prefers-reduced-motion: kill all chart entrance/transition animation.
const reduceMotion = () =>
  window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches

function render() {
  if (!chart || !props.option) return
  const opt = reduceMotion() ? { ...props.option, animation: false } : props.option
  chart.setOption(opt, true)
}
function resize() { chart && chart.resize() }

onMounted(() => {
  chart = echarts.init(el.value, null, { renderer: 'canvas' })
  render()
  window.addEventListener('resize', resize)
})
onBeforeUnmount(() => {
  window.removeEventListener('resize', resize)
  chart && chart.dispose()
  chart = null
})
watch(() => props.option, render, { deep: true })
</script>

<template>
  <div ref="el" role="img" :aria-label="ariaLabel" :style="{ width: '100%', height }"></div>
</template>
