<template>
  <div class="staking-analytics">
    <div class="controls">
      <TimeRangeSelector v-model="timeRange" />
      <Button variant="ghost" size="sm" @click="exportData">
        💾 Export
      </Button>
    </div>

    <!-- Stats Grid -->
    <div class="stats-grid">
      <StatCard
        label="Staking Participation"
        :value="formatPercentage(metrics.participation)"
        suffix="%"
        icon="🎯"
        :loading="loading"
      />
      <StatCard
        label="Total Staked"
        :value="formatNumber(metrics.totalStaked)"
        suffix="PIV"
        icon="🔒"
        :loading="loading"
      />
      <StatCard
        label="Active Stakers"
        :value="formatNumber(metrics.activeStakers)"
        icon="👥"
        :loading="loading"
      />
      <StatCard
        label="Avg Stake Size"
        :value="formatNumber(metrics.avgStakeSize)"
        suffix="PIV"
        icon="💰"
        :loading="loading"
      />
    </div>

    <!-- Staking Participation Rate -->
    <BaseChart
      title="Staking Participation Rate Over Time"
      :option="participationOption"
      :loading="loading"
      :error="error"
      height="400px"
    />

    <!-- Block Time Variance -->
    <BaseChart
      title="Block Time Variance"
      :option="blockTimeOption"
      :loading="loading"
      :error="error"
      height="350px"
    />
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted } from 'vue'
import BaseChart from '@/components/charts/BaseChart.vue'
import TimeRangeSelector from '@/components/charts/TimeRangeSelector.vue'
import Button from '@/components/common/Button.vue'
import StatCard from '@/components/common/StatCard.vue'
import { analyticsService } from '@/services/analyticsService'
import { useChartOptions, useChartExport } from '@/composables/useCharts'
import { formatNumber, formatPercentage } from '@/utils/formatters'

const { getLineChartOption } = useChartOptions()
const { exportToCSV } = useChartExport()

const timeRange = ref('30d')
const loading = ref(false)
const error = ref(null)
const stakingData = ref([])

const metrics = computed(() => {
  if (!stakingData.value || stakingData.value.length === 0) {
    return {
      participation: 0,
      totalStaked: 0,
      activeStakers: 0,
      avgStakeSize: 0
    }
  }

  const latest = stakingData.value[stakingData.value.length - 1]
  return {
    participation: latest.participationRate,
    totalStaked: Math.round(latest.totalStaked),
    activeStakers: latest.activeStakers,
    // Average size of the day's actual coinstakes (from the API), NOT
    // network-weight / stakers — that mixed total staked supply with only
    // the stakers who happened to win blocks that day.
    avgStakeSize: Math.round(latest.avgStakeSize)
  }
})

// Participation Rate Chart
const participationOption = computed(() => {
  if (!stakingData.value || stakingData.value.length === 0) {
    return getLineChartOption([], [], 'Participation %')
  }

  const dates = stakingData.value.map(d => d.date)
  const values = stakingData.value.map(d => d.participationRate)

  const option = getLineChartOption(dates, values, 'Participation Rate (%)')
  option.yAxis.axisLabel = {
    ...option.yAxis.axisLabel,
    formatter: '{value}%'
  }

  return option
})

// Block Time Variance
const blockTimeOption = computed(() => {
  if (!stakingData.value || stakingData.value.length === 0) {
    return getLineChartOption([], [], 'Block Time')
  }

  const dates = stakingData.value.map(d => d.date)
  const values = stakingData.value.map(d => d.avgBlockTime)

  const option = getLineChartOption(dates, values, 'Avg Block Time (seconds)')
  
  // Add target line at 60 seconds
  option.series.push({
    name: 'Target',
    type: 'line',
    data: dates.map(() => 60),
    lineStyle: {
      type: 'dashed',
      color: '#F59E0B',
      width: 2
    },
    itemStyle: {
      color: '#F59E0B'
    },
    symbol: 'none'
  })

  return option
})

const fetchData = async () => {
  loading.value = true
  error.value = null

  try {
    const data = await analyticsService.getStakingAnalytics(timeRange.value)

    if (data && Array.isArray(data)) {
      stakingData.value = data.map(d => ({
        date: d.date,
        // Already a percentage value from the API
        participationRate: d.participation_rate || 0,
        // Already a PIV decimal string — no satoshi conversion
        totalStaked: parseFloat(d.total_staked) || 0,
        activeStakers: d.active_stakers || 0,
        avgBlockTime: d.avg_block_time || 0,
        avgStakeSize: parseFloat(d.avg_stake_size) || 0
      }))
    } else {
      stakingData.value = []
      error.value = 'No staking analytics data available'
    }
  } catch (err) {
    error.value = 'Failed to load staking analytics. The analytics API may not be available.'
    stakingData.value = []
  } finally {
    loading.value = false
  }
}

const exportData = () => {
  if (stakingData.value && stakingData.value.length > 0) {
    exportToCSV(stakingData.value, `staking-analytics-${timeRange.value}.csv`)
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
.staking-analytics {
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

.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
  gap: var(--space-4);
}
</style>