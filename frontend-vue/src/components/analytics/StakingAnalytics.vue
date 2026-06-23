<template>
  <div class="staking-analytics">
    <div class="controls">
      <TimeRangeSelector v-model="timeRange" />
    </div>

    <div v-if="error" class="error-banner">
      <p>{{ error }}</p>
      <UiButton variant="secondary" @click="fetchData">Try Again</UiButton>
    </div>

    <template v-else>
      <!-- Summary Stats (latest day) -->
      <div class="stats-grid">
        <StatCard
          label="Staking Participation"
          :value="latest ? (latest.participationRate * 100).toFixed(2) : ''"
          format="percentage"
          :loading="loading"
        />
        <StatCard
          label="Total Staked"
          :value="latest ? formatNumber(latest.totalStaked) : ''"
          subtitle="PIV"
          :loading="loading"
        />
        <StatCard
          label="Active Stakers"
          :value="latest ? latest.activeStakers : ''"
          format="number"
          :loading="loading"
        />
        <StatCard
          label="Avg Block Time"
          :value="latest ? latest.avgBlockTime.toFixed(1) : ''"
          subtitle="seconds"
          :loading="loading"
        />
      </div>

      <!-- Participation Rate -->
      <BaseChart
        title="Staking Participation Rate Over Time"
        :option="participationOption"
        :loading="loading"
        :empty="isEmpty"
        height="400px"
      />

      <div class="chart-grid">
        <BaseChart
          title="Block Time Variance"
          :option="blockTimeOption"
          :loading="loading"
          :empty="isEmpty"
          height="350px"
        />

        <BaseChart
          title="Total Staked (PIV)"
          :option="totalStakedOption"
          :loading="loading"
          :empty="isEmpty"
          height="350px"
        />
      </div>

      <!-- Active Stakers -->
      <BaseChart
        title="Active Stakers Per Day"
        :option="stakersOption"
        :loading="loading"
        :empty="isEmpty"
        height="350px"
      />
    </template>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted } from 'vue'
import StatCard from '@/components/common/StatCard.vue'
import UiButton from '@/components/common/UiButton.vue'
import BaseChart from './BaseChart.vue'
import TimeRangeSelector from './TimeRangeSelector.vue'
import { analyticsService } from '@/services/analyticsService'
import { lineChartOption, barChartOption, referenceLineSeries } from './chartOptions'

const timeRange = ref('30d')
const loading = ref(true)
const error = ref('')
const stakingData = ref([])

const isEmpty = computed(() => !loading.value && stakingData.value.length === 0)
const latest = computed(() => stakingData.value[stakingData.value.length - 1] || null)

const formatNumber = (value) => Number(value || 0).toLocaleString(undefined, { maximumFractionDigits: 0 })

const participationOption = computed(() => {
  const dates = stakingData.value.map((d) => d.date)
  const values = stakingData.value.map((d) => Number((d.participationRate * 100).toFixed(2)))

  const option = lineChartOption(dates, values, 'Participation Rate (%)')
  option.yAxis.axisLabel = {
    ...option.yAxis.axisLabel,
    formatter: '{value}%'
  }
  return option
})

const blockTimeOption = computed(() => {
  const dates = stakingData.value.map((d) => d.date)
  const values = stakingData.value.map((d) => Number(d.avgBlockTime.toFixed(2)))

  const option = lineChartOption(dates, values, 'Avg Block Time (s)')
  if (dates.length > 0) {
    // 60 second target block time
    option.series.push(referenceLineSeries(dates, 60, 'Target (60s)'))
  }
  return option
})

const totalStakedOption = computed(() => {
  const dates = stakingData.value.map((d) => d.date)
  const values = stakingData.value.map((d) => Math.round(d.totalStaked))
  return lineChartOption(dates, values, 'Total Staked (PIV)')
})

const stakersOption = computed(() => {
  const dates = stakingData.value.map((d) => d.date)
  const values = stakingData.value.map((d) => d.activeStakers)
  return barChartOption(dates, values, 'Active Stakers')
})

const fetchData = async () => {
  loading.value = true
  error.value = ''

  try {
    const data = await analyticsService.getStakingAnalytics(timeRange.value)

    if (Array.isArray(data)) {
      stakingData.value = data.map((d) => ({
        date: d.date,
        // participation_rate is a 0-1 fraction
        participationRate: Number(d.participation_rate) || 0,
        // total_staked is a PIV decimal string
        totalStaked: parseFloat(d.total_staked) || 0,
        activeStakers: Number(d.active_stakers) || 0,
        avgBlockTime: Number(d.avg_block_time) || 0
      }))
    } else {
      stakingData.value = []
    }
  } catch (err) {
    stakingData.value = []
    error.value = 'Failed to load staking analytics.'
  } finally {
    loading.value = false
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
  justify-content: flex-end;
}

.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: var(--space-4);
}

.chart-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--space-6);
}

.error-banner {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-8);
  background: var(--bg-secondary);
  border: 2px solid var(--border-primary);
  border-radius: var(--radius-md);
  color: var(--danger);
}

@media (max-width: 768px) {
  .chart-grid {
    grid-template-columns: 1fr;
  }
}
</style>
