/* =====================================================================
   CHART THEME HELPER — shared echarts styling for the HUD aesthetic.
   Reads brand + HUD tokens from CSS so every chart stays on-palette.
   Pure presentation; never touches money units.

   Also the single echarts registration point: tree-shaken core + only
   the chart types / components actually rendered by the prototype, so
   the build ships ~400KB of echarts instead of the full ~1MB bundle.
   Every view + EChart.vue import { echarts } from here.
   ===================================================================== */
import * as echarts from 'echarts/core'
import {
  LineChart, BarChart, PieChart, ScatterChart, GaugeChart, SankeyChart,
} from 'echarts/charts'
import {
  GridComponent, TooltipComponent, LegendComponent,
  MarkLineComponent, AxisPointerComponent,
} from 'echarts/components'
import { CanvasRenderer } from 'echarts/renderers'

echarts.use([
  // charts
  LineChart, BarChart, PieChart, ScatterChart, GaugeChart, SankeyChart,
  // components (tooltip axis-trigger needs AxisPointer; markLine on interval/lorenz)
  GridComponent, TooltipComponent, LegendComponent,
  MarkLineComponent, AxisPointerComponent,
  CanvasRenderer,
])

// `echarts.init`, `echarts.graphic.LinearGradient` etc. all live on core.
export { echarts }

const css = () => getComputedStyle(document.documentElement)
export const cvar = (name) => css().getPropertyValue(name).trim()

// resolve once-per-call palette (cheap; charts rebuild on demand)
export function palette() {
  return {
    neon: cvar('--neon') || '#c46bff',
    neonSoft: cvar('--neon-soft') || '#9d4ef0',
    deep: cvar('--neon-deep') || '#662d91',
    cyan: cvar('--cyan') || '#46e6d0',
    amber: cvar('--amber') || '#ffcf5c',
    rose: cvar('--rose') || '#ff6f9c',
    hot: cvar('--hot') || '#ff5470',
    green: cvar('--success') || '#5ccb6f',
    lilac: cvar('--pv-200') || '#c9a8e8',
    axis: cvar('--text-dim') || '#8a7fa0',
    grid: 'rgba(150,90,220,0.12)',
    text: cvar('--text-muted') || '#b8b0c4',
  }
}

const FONT = 'ui-monospace, "JetBrains Mono", "SF Mono", Menlo, monospace'

// HUD-styled tooltip + base scaffolding shared by all charts.
export function baseOption(p = palette()) {
  return {
    backgroundColor: 'transparent',
    textStyle: { fontFamily: FONT },
    grid: { left: 56, right: 18, top: 30, bottom: 30, containLabel: true },
    tooltip: {
      trigger: 'axis',
      backgroundColor: 'rgba(12,7,22,0.94)',
      borderColor: 'rgba(196,107,255,0.4)',
      borderWidth: 1,
      padding: [8, 12],
      textStyle: { color: '#f4f0fa', fontFamily: FONT, fontSize: 12 },
      axisPointer: { lineStyle: { color: 'rgba(196,107,255,0.5)' }, crossStyle: { color: 'rgba(196,107,255,0.5)' } },
    },
  }
}

export function catAxis(data, p = palette(), opts = {}) {
  return {
    type: 'category',
    data,
    boundaryGap: opts.boundaryGap ?? false,
    axisLine: { lineStyle: { color: 'rgba(150,90,220,0.25)' } },
    axisTick: { show: false },
    axisLabel: { color: p.axis, fontSize: 10, fontFamily: FONT, ...(opts.axisLabel || {}) },
    ...opts.extra,
  }
}

export function valAxis(p = palette(), opts = {}) {
  return {
    type: 'value',
    scale: opts.scale ?? false,
    splitLine: { lineStyle: { color: p.grid, type: 'dashed' } },
    axisLabel: { color: p.axis, fontSize: 10, fontFamily: FONT, ...(opts.axisLabel || {}) },
    axisLine: { show: false },
    ...opts.extra,
  }
}

// vertical gradient area fill (echarts linearGradient object)
export function areaFill(echarts, hex, topA = 0.5, botA = 0.02) {
  return new echarts.graphic.LinearGradient(0, 0, 0, 1, [
    { offset: 0, color: hexA(hex, topA) },
    { offset: 1, color: hexA(hex, botA) },
  ])
}

// horizontal holographic gradient (neon -> cyan)
export function holoBar(echarts, a = '#c46bff', b = '#46e6d0') {
  return new echarts.graphic.LinearGradient(0, 0, 1, 0, [
    { offset: 0, color: a },
    { offset: 1, color: b },
  ])
}

export function hexA(hex, a) {
  const h = hex.replace('#', '')
  const n = h.length === 3 ? h.split('').map((c) => c + c).join('') : h
  const r = parseInt(n.slice(0, 2), 16)
  const g = parseInt(n.slice(2, 4), 16)
  const b = parseInt(n.slice(4, 6), 16)
  return `rgba(${r},${g},${b},${a})`
}
