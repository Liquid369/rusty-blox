import { ref } from 'vue'

/**
 * Common chart configuration for PIVX Explorer
 */
export const useChartConfig = () => {
  const colors = {
    primary: '#59FCB3',
    secondary: '#662D91',
    accent: '#C084FC',
    success: '#10B981',
    warning: '#F59E0B',
    danger: '#EF4444',
    info: '#3B82F6',
    gradient: ['#662D91', '#59FCB3']
  }

  const getBaseOption = () => ({
    tooltip: {
      trigger: 'axis',
      backgroundColor: 'rgba(17, 24, 39, 0.95)',
      borderColor: '#374151',
      borderWidth: 1,
      textStyle: {
        color: '#E5E7EB'
      }
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
        lineStyle: {
          color: '#374151'
        }
      },
      axisLabel: {
        color: '#9CA3AF'
      }
    },
    yAxis: {
      type: 'value',
      axisLine: {
        lineStyle: {
          color: '#374151'
        }
      },
      axisLabel: {
        color: '#9CA3AF'
      },
      splitLine: {
        lineStyle: {
          color: '#374151',
          type: 'dashed'
        }
      }
    }
  })

  return {
    colors,
    getBaseOption
  }
}

/**
 * Export chart data to various formats
 */
export const useChartExport = () => {
  const exportToCSV = (data, filename = 'chart-data.csv') => {
    if (!data || data.length === 0) return

    const headers = Object.keys(data[0])
    const csvContent = [
      headers.join(','),
      ...data.map(row => headers.map(header => row[header]).join(','))
    ].join('\n')

    downloadFile(csvContent, filename, 'text/csv')
  }

  const exportToJSON = (data, filename = 'chart-data.json') => {
    const jsonContent = JSON.stringify(data, null, 2)
    downloadFile(jsonContent, filename, 'application/json')
  }

  const exportChartAsPNG = (chartInstance, filename = 'chart.png') => {
    if (!chartInstance) return

    const url = chartInstance.getDataURL({
      type: 'png',
      pixelRatio: 2,
      backgroundColor: '#111827'
    })

    const link = document.createElement('a')
    link.href = url
    link.download = filename
    link.click()
  }

  const downloadFile = (content, filename, mimeType) => {
    const blob = new Blob([content], { type: mimeType })
    const url = URL.createObjectURL(blob)
    const link = document.createElement('a')
    link.href = url
    link.download = filename
    link.click()
    URL.revokeObjectURL(url)
  }

  return {
    exportToCSV,
    exportToJSON,
    exportChartAsPNG
  }
}

/**
 * Format data for time series charts
 */
export const useTimeSeriesData = () => {
  const formatTimeSeriesData = (data, timeKey = 'date', valueKey = 'value') => {
    if (!data || data.length === 0) return { dates: [], values: [] }

    const dates = data.map(item => item[timeKey])
    const values = data.map(item => item[valueKey])

    return { dates, values }
  }

  const aggregateByTimeRange = (data, range = '24h') => {
    // Implement aggregation logic based on range
    // For now, return data as-is
    return data
  }

  return {
    formatTimeSeriesData,
    aggregateByTimeRange
  }
}

/**
 * Common chart type options
 */
export const useChartOptions = () => {
  const { colors, getBaseOption } = useChartConfig()

  const getLineChartOption = (dates, values, seriesName = 'Value') => ({
    ...getBaseOption(),
    xAxis: {
      ...getBaseOption().xAxis,
      data: dates
    },
    series: [
      {
        name: seriesName,
        type: 'line',
        data: values,
        smooth: true,
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
              { offset: 0, color: 'rgba(89, 252, 179, 0.3)' },
              { offset: 1, color: 'rgba(89, 252, 179, 0)' }
            ]
          }
        }
      }
    ]
  })

  const getBarChartOption = (categories, values, seriesName = 'Value') => ({
    ...getBaseOption(),
    xAxis: {
      ...getBaseOption().xAxis,
      data: categories,
      boundaryGap: true
    },
    series: [
      {
        name: seriesName,
        type: 'bar',
        data: values,
        itemStyle: {
          color: colors.primary,
          borderRadius: [4, 4, 0, 0]
        }
      }
    ]
  })

  const getPieChartOption = (data, seriesName = 'Distribution') => ({
    tooltip: {
      trigger: 'item',
      formatter: '{b}: {c} ({d}%)',
      backgroundColor: 'rgba(17, 24, 39, 0.95)',
      borderColor: '#374151',
      textStyle: {
        color: '#E5E7EB'
      }
    },
    legend: {
      orient: 'vertical',
      left: 'left',
      textStyle: {
        color: '#9CA3AF'
      }
    },
    series: [
      {
        name: seriesName,
        type: 'pie',
        radius: ['40%', '70%'],
        avoidLabelOverlap: false,
        itemStyle: {
          borderRadius: 8,
          borderColor: '#111827',
          borderWidth: 2
        },
        label: {
          show: false
        },
        emphasis: {
          label: {
            show: true,
            fontSize: 16,
            fontWeight: 'bold',
            color: '#E5E7EB'
          }
        },
        data: data
      }
    ]
  })

  return {
    getLineChartOption,
    getBarChartOption,
    getPieChartOption
  }
}
