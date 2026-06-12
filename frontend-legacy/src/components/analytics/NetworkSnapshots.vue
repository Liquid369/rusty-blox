<template>
  <div class="network-snapshots">
    <div class="controls">
      <p class="controls-note">Periodic network snapshots over the last 7 days</p>
      <Button variant="ghost" size="sm" @click="exportData">
        💾 Export
      </Button>
    </div>

    <!-- Empty state: collection just started, nothing recorded yet -->
    <EmptyState
      v-if="!loading && !error && snapshots.length === 0"
      icon="📡"
      title="Collecting Network Snapshots"
      message="Snapshot collection has just started. Mempool, masternode and supply metrics are recorded periodically — check back soon."
    />

    <template v-else>
      <!-- Sparse data notice -->
      <div v-if="isSparse" class="sparse-notice">
        <Badge variant="info">Collecting</Badge>
        <span>
          Collecting since {{ collectingSince }} — {{ snapshots.length }}
          {{ snapshots.length === 1 ? 'snapshot' : 'snapshots' }} recorded so far. Charts will fill in over time.
        </span>
      </div>

      <!-- Summary Stats (latest snapshot) -->
      <div class="stats-grid">
        <StatCard
          label="Masternodes"
          :value="formatNumber(stats.masternodeCount)"
          icon="🖥️"
          :loading="loading"
        />
        <StatCard
          label="Mempool Transactions"
          :value="formatNumber(stats.mempoolTxs)"
          icon="⏳"
          :loading="loading"
        />
        <StatCard
          label="Mempool Size"
          :value="formatBytes(stats.mempoolBytes)"
          icon="📦"
          :loading="loading"
        />
        <StatCard
          label="Shield Supply"
          :value="formatBalance(stats.shieldSupply)"
          suffix="PIV"
          icon="🛡️"
          :loading="loading"
        />
      </div>

      <!-- Mempool Depth -->
      <BaseChart
        title="Mempool Depth"
        :option="mempoolOption"
        :loading="loading"
        :error="error"
        height="350px"
      />

      <!-- Masternodes + Shield Supply -->
      <div class="chart-grid">
        <BaseChart
          title="Masternode Count"
          :option="masternodeOption"
          :loading="loading"
          :error="error"
          height="350px"
        />

        <BaseChart
          title="Shield Supply"
          :option="shieldOption"
          :loading="loading"
          :error="error"
          height="350px"
        />
      </div>
    </template>
  </div>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import BaseChart from '@/components/charts/BaseChart.vue'
import Button from '@/components/common/Button.vue'
import Badge from '@/components/common/Badge.vue'
import StatCard from '@/components/common/StatCard.vue'
import EmptyState from '@/components/common/EmptyState.vue'
import api from '@/services/api'
import { useChartOptions, useChartExport } from '@/composables/useCharts'
import { formatNumber, formatBytes, formatDate } from '@/utils/formatters'

const { getLineChartOption } = useChartOptions()
const { exportToCSV } = useChartExport()

const SNAPSHOT_HOURS = 168
// Below this many points the series is still "filling in" — be honest about it
const SPARSE_THRESHOLD = 24

const loading = ref(false)
const error = ref(null)
const snapshots = ref([])

// Format a balance that is ALREADY in PIV (API returns PIV decimal numbers)
const formatBalance = (piv) => {
  const n = Number(piv)
  if (!isFinite(n)) return '0.00'
  return n.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })
}

const isSparse = computed(() =>
  snapshots.value.length > 0 && snapshots.value.length < SPARSE_THRESHOLD
)

const collectingSince = computed(() =>
  snapshots.value.length > 0 ? formatDate(snapshots.value[0].ts) : ''
)

const stats = computed(() => {
  if (!snapshots.value || snapshots.value.length === 0) {
    return {
      masternodeCount: 0,
      mempoolTxs: 0,
      mempoolBytes: 0,
      shieldSupply: 0
    }
  }

  const latest = snapshots.value[snapshots.value.length - 1]
  return {
    masternodeCount: latest.masternodeCount,
    mempoolTxs: latest.mempoolTxs,
    mempoolBytes: latest.mempoolBytes,
    shieldSupply: latest.shieldSupply
  }
})

const timeLabels = computed(() =>
  snapshots.value.map(s =>
    new Date(s.ts * 1000).toLocaleString('en-US', {
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit'
    })
  )
)

const mempoolOption = computed(() => {
  if (snapshots.value.length === 0) {
    return getLineChartOption([], [], 'Mempool Transactions')
  }

  const option = getLineChartOption(
    timeLabels.value,
    snapshots.value.map(s => s.mempoolTxs),
    'Mempool Transactions'
  )
  // With only a few snapshots an area line is invisible — show the points too
  option.series[0].showSymbol = true
  return option
})

const masternodeOption = computed(() => {
  if (snapshots.value.length === 0) {
    return getLineChartOption([], [], 'Masternodes')
  }

  const option = getLineChartOption(
    timeLabels.value,
    snapshots.value.map(s => s.masternodeCount),
    'Masternodes'
  )
  option.series[0].showSymbol = true
  option.yAxis.min = 'dataMin'
  return option
})

const shieldOption = computed(() => {
  if (snapshots.value.length === 0) {
    return getLineChartOption([], [], 'Shield Supply (PIV)')
  }

  const option = getLineChartOption(
    timeLabels.value,
    snapshots.value.map(s => Math.round(s.shieldSupply * 100) / 100),
    'Shield Supply (PIV)'
  )
  option.series[0].showSymbol = true
  option.yAxis.min = 'dataMin'
  return option
})

const fetchData = async () => {
  loading.value = true
  error.value = null

  try {
    const response = await api.get('/api/v2/analytics/snapshots', {
      params: { hours: SNAPSHOT_HOURS }
    })
    const data = response.data

    if (data && Array.isArray(data)) {
      snapshots.value = data.map(s => ({
        ts: s.ts,
        mempoolTxs: s.mempool_txs || 0,
        mempoolBytes: s.mempool_bytes || 0,
        masternodeCount: s.masternode_count || 0,
        // Already PIV decimal numbers — no satoshi conversion
        shieldSupply: s.shield_supply_piv || 0,
        transparentSupply: s.transparent_supply_piv || 0
      }))
    } else {
      snapshots.value = []
      error.value = 'No snapshot data available'
    }
  } catch (err) {
    error.value = 'Failed to load network snapshots. The analytics API may not be available.'
    snapshots.value = []
  } finally {
    loading.value = false
  }
}

const exportData = () => {
  if (snapshots.value && snapshots.value.length > 0) {
    exportToCSV(snapshots.value, 'network-snapshots.csv')
  }
}

onMounted(() => {
  fetchData()
})
</script>

<style scoped>
.network-snapshots {
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

.controls-note {
  margin: 0;
  font-size: var(--text-sm);
  color: var(--text-secondary);
}

.sparse-notice {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-4) var(--space-6);
  background: rgba(var(--rgb-purple-dark), 0.3);
  border: 1px dashed var(--border-secondary);
  border-radius: var(--radius-lg);
  font-size: var(--text-sm);
  color: var(--text-secondary);
}

.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
  gap: var(--space-4);
}

.chart-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--space-6);
}

@media (max-width: 768px) {
  .chart-grid {
    grid-template-columns: 1fr;
  }
}
</style>
