<script setup>
/* =====================================================================
   Minimal echarts wrapper. Pass an `option` object; it inits, updates on
   change, and resizes with the container. NEUTRAL: no baked theme beyond
   transparent background; prototypes pass brand colors via the option
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

let ro = null
let raf = 0

// Legend icons and tooltip dots take the SERIES color, which falls back to the
// echarts palette when a series only sets lineStyle.color; the line and its
// legend entry then disagree. Promote a plain lineStyle color to the series
// color unless the series already sets one.
function alignSeriesColors(opt) {
  if (!Array.isArray(opt.series)) return opt
  const series = opt.series.map((s) => {
    const c = s && s.lineStyle && s.lineStyle.color
    if (typeof c !== 'string' || s.color || (s.itemStyle && s.itemStyle.color)) return s
    return { ...s, color: c }
  })
  return { ...opt, series }
}

function render() {
  if (!chart || !props.option) return
  const base = alignSeriesColors(props.option)
  const opt = reduceMotion() ? { ...base, animation: false } : base
  chart.setOption(opt, true)
}
// rAF-throttled: observe the CONTAINER (not just window) so the canvas tracks any
// width change: breakpoint reflow, a sidebar toggle, or a Fold device folding/
// unfolding. window.resize alone left the canvas stale and overflowing.
function resize() {
  if (!chart) return
  cancelAnimationFrame(raf)
  raf = requestAnimationFrame(() => chart && chart.resize())
}

onMounted(() => {
  chart = echarts.init(el.value, null, { renderer: 'canvas' })
  render()
  if (window.ResizeObserver) {
    ro = new ResizeObserver(resize)
    ro.observe(el.value)
  } else {
    window.addEventListener('resize', resize)
  }
})
onBeforeUnmount(() => {
  cancelAnimationFrame(raf)
  if (ro) ro.disconnect()
  else window.removeEventListener('resize', resize)
  chart && chart.dispose()
  chart = null
})
watch(() => props.option, render, { deep: true })
</script>

<template>
  <div ref="el" role="img" :aria-label="ariaLabel" :style="{ width: '100%', minWidth: 0, height }"></div>
</template>
