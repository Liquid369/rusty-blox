<template>
  <div class="chart-container">
    <div v-if="title" class="chart-header">
      <h3 class="chart-title">{{ title }}</h3>
      <div class="chart-actions">
        <slot name="actions" />
      </div>
    </div>
    <div class="chart-wrapper" :style="{ height: height }">
      <v-chart
        v-if="!loading && !error"
        :option="chartOption"
        :theme="theme"
        :autoresize="true"
        @click="handleClick"
      />
      <div v-else-if="loading" class="chart-loading">
        <LoadingSpinner />
        <p>Loading chart data...</p>
      </div>
      <div v-else-if="error" class="chart-error">
        <p class="error-icon">⚠️</p>
        <p>{{ error }}</p>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import VChart from 'vue-echarts'
import { use } from 'echarts/core'
import { CanvasRenderer } from 'echarts/renderers'
import LoadingSpinner from '@/components/common/LoadingSpinner.vue'

import {
  LineChart,
  BarChart,
  PieChart,
  ScatterChart,
  RadarChart,
  HeatmapChart
} from 'echarts/charts'

import {
  TitleComponent,
  TooltipComponent,
  LegendComponent,
  GridComponent,
  DataZoomComponent,
  ToolboxComponent,
  VisualMapComponent
} from 'echarts/components'

// Register ECharts components
use([
  CanvasRenderer,
  LineChart,
  BarChart,
  PieChart,
  ScatterChart,
  RadarChart,
  HeatmapChart,
  TitleComponent,
  TooltipComponent,
  LegendComponent,
  GridComponent,
  DataZoomComponent,
  ToolboxComponent,
  VisualMapComponent
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
    default: null
  },
  theme: {
    type: String,
    default: 'dark'
  }
})

const emit = defineEmits(['click'])

const chartOption = computed(() => ({
  ...props.option,
  backgroundColor: 'transparent',
  textStyle: {
    color: '#E5E7EB'
  }
}))

const handleClick = (params) => {
  emit('click', params)
}
</script>

<style scoped>
.chart-container {
  background: var(--card-bg);
  border-radius: var(--radius-lg);
  padding: var(--space-6);
  border: 1px solid var(--border-color);
}

.chart-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--space-4);
}

.chart-title {
  margin: 0;
  font-size: var(--text-xl);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
}

.chart-actions {
  display: flex;
  gap: var(--space-2);
}

.chart-wrapper {
  position: relative;
  width: 100%;
}

.chart-loading,
.chart-error {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: var(--text-secondary);
}

.chart-loading p {
  margin-top: var(--space-3);
}

.error-icon {
  font-size: 3rem;
  margin-bottom: var(--space-2);
}

.chart-error p:last-child {
  color: var(--danger);
}
</style>
