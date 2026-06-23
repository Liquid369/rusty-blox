<template>
  <div class="kpi-band">
    <!-- Loading skeletons -->
    <template v-if="loading">
      <div v-for="i in 12" :key="`kpi-sk-${i}`" class="kpi-tile kpi-skeleton">
        <SkeletonLoader variant="text" width="60%" />
        <SkeletonLoader variant="title" width="80%" />
      </div>
    </template>

    <!-- Error state (all sources failed) -->
    <div v-else-if="allFailed" class="kpi-error">
      <p><Icon name="alert-triangle" :size="14" /> Failed to load network statistics</p>
    </div>

    <!-- Tiles -->
    <template v-else>
      <component
        :is="tile.to ? RouterLink : 'div'"
        v-for="tile in tiles"
        :key="tile.key"
        v-bind="tile.to ? { to: tile.to } : {}"
        class="kpi-tile"
        :class="{ 'kpi-link': !!tile.to }"
      >
        <div class="kpi-label">{{ tile.label }}</div>
        <div class="kpi-value">
          {{ tile.value }}<span v-if="tile.unit" class="kpi-unit">{{ tile.unit }}</span>
        </div>
        <div v-if="tile.sub" class="kpi-sub">{{ tile.sub }}</div>
      </component>
    </template>
  </div>
</template>

<script setup>
import Icon from '@/components/common/Icon.vue'
import { computed, onMounted, ref } from 'vue'
import { RouterLink } from 'vue-router'
import api from '@/services/api'
import { analyticsService } from '@/services/analyticsService'
import { formatNumber } from '@/utils/formatters'
import SkeletonLoader from '@/components/common/SkeletonLoader.vue'

const loading = ref(true)
const txDay = ref(null) // last complete day of /analytics/transactions
const stakingDay = ref(null) // last complete day of /analytics/staking
const networkDay = ref(null) // last complete day of /analytics/network
const mnCount = ref(null) // /mncount
const supply = ref(null) // /moneysupply
const price = ref(null) // /price

const allFailed = computed(() => {
  return !txDay.value && !stakingDay.value && !networkDay.value && !mnCount.value && !supply.value && !price.value
})

/**
 * Pick the most recent COMPLETE day (today's UTC bucket is still accumulating).
 */
function lastCompleteDay(rows) {
  if (!Array.isArray(rows) || rows.length === 0) return null
  const today = new Date().toISOString().slice(0, 10)
  const sorted = [...rows].sort((a, b) => String(a.date).localeCompare(String(b.date)))
  const complete = sorted.filter((r) => String(r.date) < today)
  return complete.length ? complete[complete.length - 1] : sorted[sorted.length - 1]
}

/** Compact number: 1234567 -> "1.23M" */
function compact(n, digits = 2) {
  const num = typeof n === 'string' ? parseFloat(n) : n
  if (num === null || num === undefined || isNaN(num)) return '—'
  const abs = Math.abs(num)
  if (abs >= 1e9) return `${(num / 1e9).toFixed(digits)}B`
  if (abs >= 1e6) return `${(num / 1e6).toFixed(digits)}M`
  if (abs >= 1e4) return `${(num / 1e3).toFixed(digits)}K`
  return num.toLocaleString('en-US', { maximumFractionDigits: digits })
}

const tiles = computed(() => {
  const out = []
  const day = txDay.value
  const stk = stakingDay.value
  const usd = price.value?.usd || 0

  if (day) {
    // Analytics fields: count, volume (PIV string), payment_count, stake_count,
    // avg_fee (PIV string, per payment), active_addresses, new_addresses, sapling_txs
    const volume = parseFloat(day.volume)
    const avgFee = parseFloat(day.avg_fee)
    const totalFees = isNaN(avgFee) ? null : avgFee * (day.payment_count || 0)

    out.push({
      key: 'txs',
      label: '24h Transactions',
      value: formatNumber(day.count),
      sub: `${formatNumber(day.payment_count)} payments · ${formatNumber(day.stake_count)} stakes`,
      to: '/analytics'
    })
    out.push({
      key: 'volume',
      label: '24h Volume',
      value: compact(volume),
      unit: 'PIV',
      sub: usd > 0 && !isNaN(volume) ? `≈ $${compact(volume * usd)}` : '',
      to: '/analytics'
    })
    out.push({
      key: 'fees',
      label: '24h Fees',
      value: totalFees === null ? '—' : compact(totalFees),
      unit: totalFees === null ? '' : 'PIV',
      sub: isNaN(avgFee) ? '' : `avg ${compact(avgFee)} / payment`,
      to: '/analytics'
    })
    out.push({
      key: 'active',
      label: 'Active Addresses',
      value: formatNumber(day.active_addresses),
      sub: 'last 24h',
      to: '/analytics'
    })
    out.push({
      key: 'new',
      label: 'New Addresses',
      value: formatNumber(day.new_addresses),
      sub: 'first seen 24h',
      to: '/analytics'
    })
    out.push({
      key: 'sapling',
      label: 'Sapling Txs',
      value: formatNumber(day.sapling_txs),
      sub: 'shielded · 24h',
      to: '/analytics'
    })
  }

  if (networkDay.value) {
    // Network analytics fields: blocks_per_day, interval_p95_secs, difficulty
    const net = networkDay.value
    out.push({
      key: 'blocks',
      label: '24h Blocks',
      value: formatNumber(net.blocks_per_day),
      sub: net.interval_p95_secs ? `p95 interval ${net.interval_p95_secs}s` : '',
      to: '/analytics'
    })
  }

  if (stk) {
    // total_staked is a PIV-denominated string; participation_rate / apy_estimate are %
    out.push({
      key: 'staked',
      label: 'Staked',
      value: compact(stk.total_staked),
      unit: 'PIV',
      sub: `${Number(stk.participation_rate || 0).toFixed(1)}% participation`,
      to: '/analytics'
    })
    out.push({
      key: 'apy',
      label: 'Staking APY',
      value: `${Number(stk.apy_estimate || 0).toFixed(2)}%`,
      sub: `${formatNumber(stk.active_stakers)} active stakers`,
      to: '/analytics'
    })
  }

  if (mnCount.value) {
    out.push({
      key: 'mn',
      label: 'Masternodes',
      value: formatNumber(mnCount.value.total),
      sub: `${formatNumber(mnCount.value.enabled)} enabled`
    })
  }

  if (supply.value) {
    out.push({
      key: 'supply',
      label: 'Supply',
      value: compact(supply.value.moneysupply),
      unit: 'PIV',
      sub: `${compact(supply.value.shieldsupply)} shielded`
    })
  }

  // PIV price lives in the header, so the band shows network difficulty here
  // instead (no extra fetch — networkDay is already loaded for '24h Blocks').
  if (networkDay.value?.difficulty != null) {
    const diff = parseFloat(networkDay.value.difficulty)
    out.push({
      key: 'difficulty',
      label: 'Difficulty',
      value: isNaN(diff) ? '—' : compact(diff),
      sub: 'PoS network weight'
    })
  }

  return out
})

onMounted(async () => {
  const [txRes, stakingRes, networkRes, mnRes, supplyRes, priceRes] = await Promise.allSettled([
    analyticsService.getTransactionAnalytics('7d'),
    analyticsService.getStakingAnalytics('7d'),
    analyticsService.getNetworkHealth('7d'),
    api.get('/api/v2/mncount'),
    api.get('/api/v2/moneysupply'),
    api.get('/api/v2/price')
  ])

  if (txRes.status === 'fulfilled') txDay.value = lastCompleteDay(txRes.value)
  if (stakingRes.status === 'fulfilled') stakingDay.value = lastCompleteDay(stakingRes.value)
  if (networkRes.status === 'fulfilled') networkDay.value = lastCompleteDay(networkRes.value)
  if (mnRes.status === 'fulfilled') mnCount.value = mnRes.value.data
  if (supplyRes.status === 'fulfilled') supply.value = supplyRes.value.data
  if (priceRes.status === 'fulfilled') price.value = priceRes.value.data

  loading.value = false
})
</script>

<style scoped>
/* 12 tiles: column counts (6/4/3/2) all divide 12, so rows always balance */
.kpi-band {
  display: grid;
  grid-template-columns: repeat(6, minmax(0, 1fr));
  gap: var(--space-3);
}

@media (max-width: 1280px) {
  .kpi-band {
    grid-template-columns: repeat(4, minmax(0, 1fr));
  }
}

@media (max-width: 980px) {
  .kpi-band {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }
}

.kpi-tile {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  padding: var(--space-4);
  border-radius: var(--radius-md);
  background: var(--glass-bg);
  border: 1px solid var(--glass-border);
  backdrop-filter: blur(var(--blur-sm));
  -webkit-backdrop-filter: blur(var(--blur-sm));
  box-shadow: var(--shadow-xs), var(--glass-highlight);
  text-decoration: none;
  min-height: 92px;
  transition:
    transform var(--transition-base),
    border-color var(--transition-base),
    box-shadow var(--transition-base);
}

.kpi-link {
  cursor: pointer;
}

.kpi-link:hover {
  transform: translateY(-2px);
  border-color: var(--glass-border-hover);
  box-shadow: var(--shadow-sm), var(--glow-purple), var(--glass-highlight);
}

.kpi-link:focus-visible {
  outline: 2px solid var(--focus-ring-color);
  outline-offset: 2px;
}

.kpi-label {
  font-size: var(--text-2xs);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
  color: var(--text-secondary);
  font-weight: var(--weight-bold);
}

.kpi-value {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
  font-size: var(--text-xl);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
  line-height: 1.2;
  display: flex;
  align-items: baseline;
  gap: var(--space-1);
  flex-wrap: wrap;
}

.kpi-unit {
  font-family: var(--font-primary);
  font-size: var(--text-xs);
  font-weight: var(--weight-semibold);
  color: var(--text-tertiary);
  text-transform: uppercase;
}

.kpi-sub {
  font-size: var(--text-xs);
  color: var(--text-tertiary);
  font-variant-numeric: tabular-nums;
  margin-top: auto;
}

.kpi-skeleton {
  justify-content: center;
}

.kpi-error {
  grid-column: 1 / -1;
  text-align: center;
  color: var(--text-tertiary);
  font-style: italic;
  padding: var(--space-6);
}

@media (max-width: 768px) {
  .kpi-band {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-2);
  }

  .kpi-tile {
    padding: var(--space-3);
    min-height: 80px;
  }

  .kpi-value {
    font-size: var(--text-lg);
  }
}
</style>
