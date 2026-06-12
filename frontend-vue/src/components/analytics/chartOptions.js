/**
 * Shared ECharts option builders themed to the PIVX brand palette
 * (see src/assets/styles/variables.css).
 */

export const chartColors = {
  accent: '#59FCB3',
  accentDark: '#3DD99B',
  purple: '#662D91',
  purpleLight: '#8547B3',
  warning: '#F59E0B',
  danger: '#EF4444',
  info: '#59B3FC',
  axisLine: '#4D3077',
  axisLabel: '#8B8B8B',
  splitLine: '#2A1B42',
  tooltipBg: 'rgba(19, 13, 30, 0.95)'
}

const tooltipStyle = {
  backgroundColor: chartColors.tooltipBg,
  borderColor: chartColors.axisLine,
  borderWidth: 1,
  textStyle: {
    color: '#FFFFFF'
  }
}

export const baseOption = () => ({
  tooltip: {
    trigger: 'axis',
    ...tooltipStyle
  },
  grid: {
    left: '3%',
    right: '4%',
    bottom: '3%',
    top: '10%',
    containLabel: true
  },
  xAxis: {
    type: 'category',
    boundaryGap: false,
    axisLine: {
      lineStyle: { color: chartColors.axisLine }
    },
    axisLabel: { color: chartColors.axisLabel }
  },
  yAxis: {
    type: 'value',
    axisLine: {
      lineStyle: { color: chartColors.axisLine }
    },
    axisLabel: { color: chartColors.axisLabel },
    splitLine: {
      lineStyle: { color: chartColors.splitLine, type: 'dashed' }
    }
  }
})

export const lineChartOption = (dates, values, seriesName = 'Value') => {
  const option = baseOption()
  return {
    ...option,
    xAxis: { ...option.xAxis, data: dates },
    series: [
      {
        name: seriesName,
        type: 'line',
        data: values,
        smooth: true,
        lineStyle: { color: chartColors.accent, width: 2 },
        itemStyle: { color: chartColors.accent },
        areaStyle: {
          color: {
            type: 'linear',
            x: 0,
            y: 0,
            x2: 0,
            y2: 1,
            colorStops: [
              { offset: 0, color: 'rgba(89, 252, 179, 0.3)' },
              { offset: 1, color: 'rgba(89, 252, 179, 0)' }
            ]
          }
        }
      }
    ]
  }
}

export const barChartOption = (categories, values, seriesName = 'Value') => {
  const option = baseOption()
  return {
    ...option,
    xAxis: { ...option.xAxis, data: categories, boundaryGap: true },
    series: [
      {
        name: seriesName,
        type: 'bar',
        data: values,
        itemStyle: {
          color: chartColors.purpleLight,
          borderRadius: [4, 4, 0, 0]
        }
      }
    ]
  }
}

export const pieChartOption = (data, seriesName = 'Distribution') => ({
  tooltip: {
    trigger: 'item',
    formatter: '{b}: {c} ({d}%)',
    ...tooltipStyle
  },
  legend: {
    orient: 'vertical',
    left: 'left',
    textStyle: { color: chartColors.axisLabel }
  },
  color: [
    chartColors.accent,
    chartColors.purpleLight,
    chartColors.info,
    chartColors.warning,
    chartColors.danger,
    chartColors.accentDark,
    chartColors.purple
  ],
  series: [
    {
      name: seriesName,
      type: 'pie',
      radius: ['40%', '70%'],
      avoidLabelOverlap: false,
      itemStyle: {
        borderRadius: 8,
        borderColor: '#130D1E',
        borderWidth: 2
      },
      label: { show: false },
      emphasis: {
        label: {
          show: true,
          fontSize: 16,
          fontWeight: 'bold',
          color: '#FFFFFF'
        }
      },
      data
    }
  ]
})

/**
 * Dashed horizontal reference line series (e.g. target block time).
 */
export const referenceLineSeries = (dates, value, name, color = chartColors.warning) => ({
  name,
  type: 'line',
  data: dates.map(() => value),
  lineStyle: { type: 'dashed', color, width: 2 },
  itemStyle: { color },
  symbol: 'none'
})
