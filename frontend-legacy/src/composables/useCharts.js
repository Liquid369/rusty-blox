import { ref } from 'vue'

/**
 * Common chart configuration for PIVX Explorer
 */
export const useChartConfig = () => {
  const colors = {
    primary: '#B3FF78',
    secondary: '#642D8F',
    accent: '#B359FC',
    success: '#71BB3A',
    warning: '#f6ff78',
    danger: '#EF4444',
    info: '#3B82F6',
    gradient: ['#642D8F', '#B3FF78']
  }

  const getBaseOption = () => ({
    tooltip: {
      trigger: 'axis',
      backgroundColor: 'rgba(17, 11, 27, 0.92)',
      borderColor: '#642D8F',
      borderWidth: 1,
      textStyle: {
        color: '#FFFFFF'
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
          color: '#642D8F'
        }
      },
      axisLabel: {
        color: '#9B93A8'
      }
    },
    yAxis: {
      type: 'value',
      axisLine: {
        lineStyle: {
          color: '#642D8F'
        }
      },
      axisLabel: {
        color: '#9B93A8'
      },
      splitLine: {
        lineStyle: {
          color: 'rgba(100, 45, 143, 0.45)',
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
      backgroundColor: '#110B1B'
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
              { offset: 0, color: 'rgba(179, 255, 120, 0.25)' },
              { offset: 1, color: 'rgba(179, 255, 120, 0)' }
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
      backgroundColor: 'rgba(17, 11, 27, 0.92)',
      borderColor: '#642D8F',
      textStyle: {
        color: '#FFFFFF'
      }
    },
    legend: {
      orient: 'vertical',
      left: 'left',
      textStyle: {
        color: '#9B93A8'
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
          borderColor: '#110B1B',
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
            color: '#FFFFFF'
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
