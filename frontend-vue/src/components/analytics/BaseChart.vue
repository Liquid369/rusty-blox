<template>
  <div class="chart-card">
    <div v-if="title" class="chart-header">
      <h3 class="chart-title">{{ title }}</h3>
    </div>
    <div class="chart-wrapper" :style="{ height: height }">
      <div v-if="loading" class="chart-state">
        <span class="loading-spinner"></span>
        <p>Loading chart data...</p>
      </div>
      <div v-else-if="error" class="chart-state chart-error">
        <p>{{ error }}</p>
      </div>
      <div v-else-if="empty" class="chart-state">
        <p>No data available</p>
      </div>
      <VChart v-else :option="chartOption" autoresize />
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import VChart from 'vue-echarts'
import { use } from 'echarts/core'
import { CanvasRenderer } from 'echarts/renderers'
import { LineChart, BarChart, PieChart } from 'echarts/charts'
import {
  TooltipComponent,
  LegendComponent,
  GridComponent,
  DataZoomComponent
} from 'echarts/components'

use([
  CanvasRenderer,
  LineChart,
  BarChart,
  PieChart,
  TooltipComponent,
  LegendComponent,
  GridComponent,
  DataZoomComponent
])

const props = defineProps({
  title: {
    type: String,
    default: ''
  },
  option: {
    type: Object,
    required: true
  },
  height: {
    type: String,
    default: '400px'
  },
  loading: {
    type: Boolean,
    default: false
  },
  error: {
    type: String,
    default: ''
  },
  empty: {
    type: Boolean,
    default: false
  }
})

const chartOption = computed(() => ({
  ...props.option,
  backgroundColor: 'transparent',
  textStyle: {
    color: '#B2B2B2'
  }
}))
</script>

<style scoped>
.chart-card {
  background: var(--bg-secondary);
  border: 2px solid var(--border-primary);
  border-radius: var(--radius-md);
  padding: var(--space-6);
}

.chart-header {
  margin-bottom: var(--space-4);
}

.chart-title {
  margin: 0;
  font-size: var(--text-lg);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
}

.chart-wrapper {
  position: relative;
  width: 100%;
}

.chart-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--space-3);
  height: 100%;
  color: var(--text-tertiary);
}

.chart-error p {
  color: var(--danger);
}
</style>
