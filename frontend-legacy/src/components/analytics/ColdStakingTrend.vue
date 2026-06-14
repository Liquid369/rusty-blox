<template>
  <div class="coldstaking-trend">
    <div class="controls">
      <TimeRangeSelector v-model="timeRange" :ranges="ranges" />
      <Button variant="ghost" size="sm" @click="exportData">
        <Icon name="download" :size="14" /> Export
      </Button>
    </div>

    <!-- Summary Stats -->
    <div class="stats-grid">
      <StatCard
        label="Delegated Balance"
        :value="formatBalance(stats.delegatedBalance)"
        suffix="PIV"
        subtitle="Net delegated pool"
        icon="snowflake"
        :loading="loading"
      />
      <StatCard
        label="P2CS Created"
        :value="formatBalance(stats.createdTotal)"
        suffix="PIV"
        subtitle="Turnover, incl. re-stakes"
        icon="plus"
        :loading="loading"
      />
      <StatCard
        label="P2CS Spent"
        :value="formatBalance(stats.spentTotal)"
        suffix="PIV"
        subtitle="Turnover, incl. re-stakes"
        icon="minus"
        :loading="loading"
      />
      <StatCard
        label="Net Change"
        :value="formatSignedBalance(stats.netChange)"
        suffix="PIV"
        icon="trending-up"
        :loading="loading"
      />
    </div>

    <!-- Delegated Balance + Flows Chart -->
    <BaseChart
      title="Cold Staking Delegations Over Time"
      :option="trendOption"
      :loading="loading"
      :error="error"
      height="450px"
    />
    <p class="chart-caption">
      Created / Spent are P2CS turnover and include re-stake churn (every won
      coinstake re-mints the delegation), so they run several times larger than
      net new delegation. The net delegated pool is the Delegated Balance line.
    </p>
  </div>
</template>

<script setup>
import Icon from '@/components/common/Icon.vue'
import { ref, computed, watch, onMounted } from 'vue'
import BaseChart from '@/components/charts/BaseChart.vue'
import TimeRangeSelector from '@/components/charts/TimeRangeSelector.vue'
import Button from '@/components/common/Button.vue'
import StatCard from '@/components/common/StatCard.vue'
import api from '@/services/api'
import { useChartConfig, useChartExport } from '@/composables/useCharts'

const { colors, getBaseOption } = useChartConfig()
const { exportToCSV } = useChartExport()

const ranges = [
  { value: '30d', label: '30D' },
  { value: '90d', label: '90D' },
  { value: '1y', label: '1Y' }
]

const timeRange = ref('30d')
const loading = ref(false)
const error = ref(null)
const trendData = ref([])

// Format a balance that is ALREADY in PIV (API returns PIV decimal strings)
const formatBalance = (piv) => {
  const n = Number(piv)
  if (!isFinite(n)) return '0.00'
  return n.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })
}

const formatSignedBalance = (piv) => {
  const n = Number(piv)
  if (!isFinite(n)) return '0.00'
  return `${n > 0 ? '+' : ''}${formatBalance(n)}`
}

const stats = computed(() => {
  if (!trendData.value || trendData.value.length === 0) {
    return {
      delegatedBalance: 0,
      createdTotal: 0,
      spentTotal: 0,
      netChange: 0
    }
  }

  const first = trendData.value[0]
  const latest = trendData.value[trendData.value.length - 1]

  return {
    delegatedBalance: latest.netCumulative,
    createdTotal: trendData.value.reduce((sum, d) => sum + d.created, 0),
    spentTotal: trendData.value.reduce((sum, d) => sum + d.spent, 0),
    netChange: latest.netCumulative - first.netCumulative
  }
})

// Net cumulative line (delegated balance) + created/spent bars
const trendOption = computed(() => {
  const base = getBaseOption()
  const dates = trendData.value.map(d => d.date)

  return {
    ...base,
    grid: {
      ...base.grid,
      top: '15%'
    },
    legend: {
      data: ['Delegated Balance', 'Created (incl. re-stakes)', 'Spent (incl. re-stakes)'],
      top: 0,
      textStyle: {
        color: '#9B93A8'
      }
    },
    xAxis: {
      ...base.xAxis,
      boundaryGap: true,
      data: dates
    },
    yAxis: [
      {
        ...base.yAxis,
        name: 'Delegated (PIV)',
        nameTextStyle: { color: '#9B93A8' }
      },
      {
        ...base.yAxis,
        name: 'Daily Turnover (PIV)',
        nameTextStyle: { color: '#9B93A8' },
        splitLine: { show: false }
      }
    ],
    series: [
      {
        name: 'Delegated Balance',
        type: 'line',
        yAxisIndex: 0,
        data: trendData.value.map(d => Math.round(d.netCumulative * 100) / 100),
        smooth: true,
        symbol: 'none',
        lineStyle: {
          color: colors.primary,
          width: 2
        },
        itemStyle: {
          color: colors.primary
        },
        areaStyle: {
          color: {
            type: 'linear',
            x: 0,
            y: 0,
            x2: 0,
            y2: 1,
            colorStops: [
              { offset: 0, color: 'rgba(179, 255, 120, 0.25)' },
              { offset: 1, color: 'rgba(179, 255, 120, 0)' }
            ]
          }
        }
      },
      {
        name: 'Created (incl. re-stakes)',
        type: 'bar',
        yAxisIndex: 1,
        data: trendData.value.map(d => Math.round(d.created * 100) / 100),
        itemStyle: {
          color: colors.accent,
          borderRadius: [4, 4, 0, 0]
        }
      },
      {
        name: 'Spent (incl. re-stakes)',
        type: 'bar',
        yAxisIndex: 1,
        data: trendData.value.map(d => Math.round(d.spent * 100) / 100),
        itemStyle: {
          color: colors.danger,
          borderRadius: [4, 4, 0, 0]
        }
      }
    ]
  }
})

const fetchData = async () => {
  loading.value = true
  error.value = null

  try {
    const response = await api.get('/api/v2/analytics/coldstaking', {
      params: { range: timeRange.value }
    })
    const data = response.data

    if (data && Array.isArray(data)) {
      trendData.value = data.map(d => ({
        date: d.date,
        // Already PIV decimal strings — no satoshi conversion
        created: parseFloat(d.created) || 0,
        spent: parseFloat(d.spent) || 0,
        netCumulative: parseFloat(d.net_cumulative) || 0
      }))
    } else {
      trendData.value = []
      error.value = 'No cold staking data available'
    }
  } catch (err) {
    error.value = 'Failed to load cold staking trend. The analytics API may not be available.'
    trendData.value = []
  } finally {
    loading.value = false
  }
}

const exportData = () => {
  if (trendData.value && trendData.value.length > 0) {
    exportToCSV(trendData.value, `coldstaking-trend-${timeRange.value}.csv`)
  }
}

watch(timeRange, () => {
  fetchData()
})

onMounted(() => {
  fetchData()
})
</script>

<style scoped>
.coldstaking-trend {
  display: grid;
  gap: var(--space-6);
}

.controls {
  display: flex;
  justify-content: space-between;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-3);
}

.chart-caption {
  margin: 0;
  font-size: var(--text-xs);
  line-height: 1.5;
  color: var(--text-tertiary);
}

/* 4 tiles: keep rows balanced (4 / 2x2 / 1) instead of wrapping 3+1 */
.stats-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: var(--space-4);
}

@media (max-width: 1024px) {
  .stats-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

@media (max-width: 520px) {
  .stats-grid {
    grid-template-columns: 1fr;
  }
}
</style>
