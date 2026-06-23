<template>
  <div class="supply-analytics">
    <div class="controls">
      <Button variant="ghost" size="sm" @click="exportData">
        <Icon name="download" :size="14" /> Export
      </Button>
    </div>

    <!-- Money Supply Over Time (only when the API provides historical data) -->
    <BaseChart
      v-if="historicalData.length > 0"
      title="Money Supply Over Time"
      :option="moneySupplyOption"
      :loading="loading"
      :error="error"
      height="400px"
    />

    <!-- Supply composition: a stacked area once a historical series exists,
         otherwise one honest proportion bar for the current snapshot. (The old
         two-bars-on-one-date chart read as a fake trend and duplicated both the
         distribution pie and the metrics card — all three showed the same pair
         of numbers.) -->
    <div class="chart-grid">
      <BaseChart
        title="Supply Composition"
        :option="compositionOption"
        :loading="loading"
        :error="error"
        height="240px"
      />

      <Card class="stats-card">
        <h3>Supply Metrics</h3>
        <div v-if="loading" class="loading-state">Loading...</div>
        <div v-else-if="error" class="error-state">{{ error }}</div>
        <div v-else-if="!supplyData" class="empty-state">No data available</div>
        <div v-else class="metrics">
          <div class="metric">
            <span class="label">Total Supply</span>
            <span class="value">{{ formatNumber(supplyData.totalSupply) }} PIV</span>
          </div>
          <div class="metric">
            <span class="label">Transparent</span>
            <span class="value">{{ formatNumber(supplyData.transparent) }} PIV</span>
          </div>
          <div class="metric">
            <span class="label">Shielded</span>
            <span class="value">{{ formatNumber(supplyData.shielded) }} PIV</span>
          </div>
          <div class="metric">
            <span class="label">Shield Adoption</span>
            <span class="value accent">{{ formatPercentage(shieldedPercentage) }}%</span>
          </div>
        </div>
      </Card>
    </div>
  </div>
</template>

<script setup>
import Icon from '@/components/common/Icon.vue'
import { ref, computed, onMounted } from 'vue'
import BaseChart from '@/components/charts/BaseChart.vue'
import Button from '@/components/common/Button.vue'
import Card from '@/components/common/Card.vue'
import { analyticsService } from '@/services/analyticsService'
import { useChartOptions, useChartExport } from '@/composables/useCharts'
import { formatNumber, formatPercentage } from '@/utils/formatters'

const { getLineChartOption } = useChartOptions()
const { exportToCSV } = useChartExport()

const timeRange = ref('30d')
const loading = ref(false)
const error = ref(null)
const supplyData = ref(null)
const historicalData = ref([])

const shieldedPercentage = computed(() => {
  if (!supplyData.value) return 0
  // Provided directly by the API — do not recompute
  return supplyData.value.shieldAdoption
})

// Money Supply Chart Option (chart is hidden in template when no historical data)
const moneySupplyOption = computed(() => {
  if (!historicalData.value || historicalData.value.length === 0) {
    return getLineChartOption([], [], 'Total Supply')
  }

  const dates = historicalData.value.map(d => d.date)
  const values = historicalData.value.map(d => d.totalSupply)

  return getLineChartOption(dates, values, 'Total Supply (PIV)')
})

// Supply composition. A stacked area over time once the API returns a historical
// series; until then a single proportion bar for the current snapshot — honest
// for one data point, where two vertical bars only implied a trend that isn't
// there. Brand greens for transparent, brand purples for shielded.
const compositionOption = computed(() => {
  // Historical series (future-proof) — stacked area over time.
  if (historicalData.value && historicalData.value.length > 0) {
    const dates = historicalData.value.map(d => d.date)
    return {
      tooltip: {
        trigger: 'axis',
        backgroundColor: 'rgba(17, 11, 27, 0.92)',
        borderColor: '#642D8F',
        textStyle: { color: '#FFFFFF' }
      },
      legend: { data: ['Transparent', 'Shielded'], textStyle: { color: '#9B93A8' }, bottom: 0 },
      grid: { left: '3%', right: '4%', bottom: '14%', top: '8%', containLabel: true },
      xAxis: {
        type: 'category',
        data: dates,
        axisLine: { lineStyle: { color: '#642D8F' } },
        axisLabel: { color: '#9B93A8' }
      },
      yAxis: {
        type: 'value',
        axisLine: { lineStyle: { color: '#642D8F' } },
        axisLabel: { color: '#9B93A8' },
        splitLine: { lineStyle: { color: 'rgba(100, 45, 143, 0.45)', type: 'dashed' } }
      },
      series: [
        {
          name: 'Transparent',
          type: 'line',
          stack: 'Total',
          smooth: true,
          showSymbol: false,
          areaStyle: { color: 'rgba(179, 255, 120, 0.25)' },
          lineStyle: { color: '#B3FF78' },
          data: historicalData.value.map(d => d.transparent)
        },
        {
          name: 'Shielded',
          type: 'line',
          stack: 'Total',
          smooth: true,
          showSymbol: false,
          areaStyle: { color: 'rgba(179, 89, 252, 0.18)' },
          lineStyle: { color: '#B359FC' },
          data: historicalData.value.map(d => d.shielded)
        }
      ]
    }
  }

  // Single snapshot — one horizontal proportion bar (transparent vs shielded).
  if (!supplyData.value) return { series: [] }
  const t = supplyData.value.transparent
  const s = supplyData.value.shielded
  const total = t + s
  const pct = (v) => (total > 0 ? (v / total) * 100 : 0)

  return {
    grid: { left: 12, right: 12, top: 22, bottom: 48 },
    tooltip: {
      trigger: 'item',
      backgroundColor: 'rgba(17, 11, 27, 0.92)',
      borderColor: '#642D8F',
      textStyle: { color: '#FFFFFF' },
      formatter: (p) => `${p.marker} ${p.seriesName}<br/><b>${formatNumber(p.value)} PIV</b> · ${pct(p.value).toFixed(2)}%`
    },
    legend: {
      bottom: 0,
      icon: 'roundRect',
      itemWidth: 12,
      itemHeight: 12,
      data: ['Transparent', 'Shielded'],
      textStyle: { color: '#C9C2D6' },
      formatter: (name) => `${name}  ${pct(name === 'Transparent' ? t : s).toFixed(2)}%`
    },
    xAxis: { type: 'value', max: total, show: false },
    yAxis: { type: 'category', data: ['supply'], show: false },
    series: [
      {
        name: 'Transparent',
        type: 'bar',
        stack: 'supply',
        barWidth: 64,
        itemStyle: {
          borderRadius: [10, 0, 0, 10],
          color: { type: 'linear', x: 0, y: 0, x2: 1, y2: 0, colorStops: [{ offset: 0, color: '#9BE85C' }, { offset: 1, color: '#B3FF78' }] }
        },
        label: {
          show: true,
          position: 'insideLeft',
          color: '#0c0717',
          fontWeight: 'bold',
          fontSize: 14,
          formatter: () => `${pct(t).toFixed(2)}%`
        },
        data: [t]
      },
      {
        name: 'Shielded',
        type: 'bar',
        stack: 'supply',
        barWidth: 64,
        itemStyle: {
          borderRadius: [0, 10, 10, 0],
          color: { type: 'linear', x: 0, y: 0, x2: 1, y2: 0, colorStops: [{ offset: 0, color: '#8B3FE0' }, { offset: 1, color: '#B359FC' }] }
        },
        data: [s]
      }
    ]
  }
})

const fetchData = async () => {
  loading.value = true
  error.value = null

  try {
    const data = await analyticsService.getSupplyAnalytics(timeRange.value)

    if (data && data.current) {
      // Parse API response (snake_case with string PIV amounts)
      supplyData.value = {
        totalSupply: parseFloat(data.current.total_supply) || 0,
        transparent: parseFloat(data.current.transparent_supply) || 0,
        shielded: parseFloat(data.current.shielded_supply) || 0,
        shieldAdoption: data.current.shield_adoption_percentage || 0
      }

      // Parse historical data if available
      if (data.historical && data.historical.length > 0) {
        historicalData.value = data.historical.map(point => ({
          date: point.date,
          totalSupply: parseFloat(point.total) || 0,
          transparent: parseFloat(point.transparent) || 0,
          shielded: parseFloat(point.shielded) || 0
        }))
      } else {
        // No historical data available - use current value for display
        historicalData.value = []
      }
    } else {
      throw new Error('Invalid API response')
    }
  } catch (err) {
    error.value = 'Failed to load supply data from API.'
    // Set empty data on error
    supplyData.value = null
    historicalData.value = []
  } finally {
    loading.value = false
  }
}

const exportData = () => {
  if (historicalData.value && historicalData.value.length > 0) {
    exportToCSV(historicalData.value, `supply-analytics-${timeRange.value}.csv`)
  } else if (supplyData.value) {
    // Export current snapshot if no historical data
    const today = new Date().toISOString().split('T')[0]
    exportToCSV([{
      date: today,
      totalSupply: supplyData.value.totalSupply,
      transparent: supplyData.value.transparent,
      shielded: supplyData.value.shielded
    }], `supply-snapshot-${today}.csv`)
  }
}

onMounted(() => {
  fetchData()
})
</script>

<style scoped>
.supply-analytics {
  display: grid;
  gap: var(--space-6);
}

.controls {
  display: flex;
  justify-content: flex-end;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-3);
}

.chart-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--space-6);
}

.stats-card {
  padding: var(--space-6);
}

.stats-card h3 {
  margin-bottom: var(--space-4);
  color: var(--text-primary);
}

.metrics {
  display: grid;
  gap: var(--space-4);
}

.metric {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--space-3);
  background: rgba(255, 255, 255, 0.03);
  border-radius: var(--radius-md);
}

.metric .label {
  color: var(--text-secondary);
  font-size: var(--text-sm);
}

.metric .value {
  font-weight: var(--weight-bold);
  color: var(--text-primary);
}

.metric .value.accent {
  color: var(--text-accent);
}

@media (max-width: 768px) {
  .chart-grid {
    grid-template-columns: 1fr;
  }
}
</style>
