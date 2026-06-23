<template>
  <tr class="block-row">
    <td class="cell cell-height">
      <router-link :to="`/block/${block.height}`" class="height-link">
        #{{ formatNumber(block.height) }}
      </router-link>
    </td>
    <td class="cell cell-age">{{ age }}</td>
    <td class="cell cell-num">{{ formatNumber(block.txCount) }}</td>
    <td class="cell cell-types">
      <div v-if="chips.length" class="chip-row">
        <Badge
          v-for="chip in chips"
          :key="chip.key"
          :variant="chip.variant"
          size="sm"
          :title="chip.title"
        >
          {{ chip.count }} {{ chip.label }}
        </Badge>
      </div>
      <span v-else class="cell-muted">—</span>
    </td>
    <td class="cell cell-num cell-value">{{ totalPiv }}</td>
    <td class="cell cell-num">
      <span v-if="feesPiv !== null">{{ feesPiv }}</span>
      <span v-else class="cell-muted">—</span>
    </td>
    <td class="cell cell-num">
      <span v-if="rewardPiv !== null" class="reward-value">{{ rewardPiv }}</span>
      <span v-else class="cell-muted">—</span>
    </td>
    <td class="cell cell-num">{{ sizeKb }}</td>
    <td class="cell cell-num cell-muted-num">{{ difficultyFmt }}</td>
    <td class="cell cell-staker">
      <div v-if="block.stakers.length" class="staker-list">
        <span
          v-for="entry in block.stakers"
          :key="entry.address"
          class="staker-entry"
        >
          <span v-if="entry.role" class="staker-role" :class="`role-${entry.role.toLowerCase()}`">
            {{ entry.role }}
          </span>
          <router-link
            :to="`/address/${entry.address}`"
            class="staker-link"
            :title="entry.address"
          >
            {{ truncateHash(entry.address, 6, 4) }}
          </router-link>
        </span>
      </div>
      <span v-else class="cell-muted">—</span>
    </td>
  </tr>
</template>

<script setup>
import { computed } from 'vue'
import { formatNumber, formatPIV, formatTimeAgo, truncateHash } from '@/utils/formatters'
import Badge from '@/components/common/Badge.vue'

const props = defineProps({
  block: {
    type: Object,
    required: true
  },
  // Incremented by the parent so "age" labels recompute on a shared interval
  tick: {
    type: Number,
    default: 0
  }
})

const CHIP_META = [
  { key: 'coinstake', label: 'stake', variant: 'success', title: 'Coinstake / reward' },
  { key: 'transparent', label: 'tx', variant: 'default', title: 'Transparent' },
  { key: 'shield', label: 'shield', variant: 'info', title: 'Shield (Sapling)' },
  { key: 'coldstake', label: 'cold', variant: 'warning', title: 'Cold-stake delegation' }
]

const chips = computed(() => {
  const counts = props.block.typeCounts || {}
  return CHIP_META
    .filter(meta => (counts[meta.key] || 0) > 0)
    .map(meta => ({
      ...meta,
      count: counts[meta.key],
      title: `${counts[meta.key]} × ${meta.title}`
    }))
})

const age = computed(() => {
  // Referencing tick keeps the label fresh between new blocks
  void props.tick
  return formatTimeAgo(props.block.time)
})

const totalPiv = computed(() => {
  const sats = props.block.totalSats
  if (!sats) return '0.00'
  // formatPIV already groups with thousands-separator commas.
  return formatPIV(sats, 2)
})

const rewardPiv = computed(() => {
  const sats = props.block.rewardSats
  if (sats === null || sats === undefined) return null
  return formatPIV(sats, 2)
})

const feesPiv = computed(() => {
  const sats = props.block.feesSats
  if (!sats) return null // PoS blocks are usually fee-less; show — rather than 0
  // Fees are tiny — show up to 8 dp, trimmed of trailing zeros
  return formatPIV(sats, 8).replace(/\.?0+$/, '')
})

const sizeKb = computed(() => {
  const bytes = props.block.size
  if (!bytes || isNaN(bytes)) return '0.00'
  return (bytes / 1024).toFixed(2)
})

const difficultyFmt = computed(() => {
  const d = props.block.difficulty
  if (!d || isNaN(d)) return '—'
  return formatNumber(Math.round(d))
})
</script>

<style scoped>
.block-row {
  border-bottom: 1px solid var(--border-secondary);
  transition: background var(--transition-fast);
}

.block-row:last-child {
  border-bottom: none;
}

.block-row:hover {
  background: rgba(var(--rgb-purple-accent), 0.06);
}

.cell {
  padding: var(--space-3) var(--space-4);
  font-size: var(--text-sm);
  color: var(--text-primary);
  white-space: nowrap;
  vertical-align: middle;
}

.cell-height {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
  font-weight: var(--weight-bold);
}

.height-link {
  color: var(--text-accent);
  text-decoration: none;
  transition: color var(--transition-fast);
}

.height-link:hover {
  text-decoration: underline;
}

.cell-age {
  color: var(--text-tertiary);
  font-variant-numeric: tabular-nums;
  font-size: var(--text-xs);
}

.cell-num {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
  text-align: right;
}

.cell-value {
  font-weight: var(--weight-semibold);
}

.reward-value {
  color: var(--success);
}

.cell-muted-num {
  color: var(--text-tertiary);
}

.cell-types {
  min-width: 140px;
}

.chip-row {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  flex-wrap: nowrap;
}

.cell-muted {
  color: var(--text-tertiary);
}

.cell-staker {
  font-family: var(--font-mono);
  font-size: var(--text-xs);
}

.staker-list {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

.staker-entry {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
}

.staker-role {
  font-family: var(--font-primary);
  font-size: var(--text-2xs);
  font-weight: var(--weight-bold);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
  color: var(--text-tertiary);
}

.role-staker {
  color: var(--success);
}

.role-owner {
  color: var(--text-purple);
}

.staker-link {
  color: var(--text-purple);
  text-decoration: none;
  transition: color var(--transition-fast);
}

.staker-link:hover {
  text-decoration: underline;
}

@media (max-width: 768px) {
  .cell {
    padding: var(--space-2) var(--space-3);
  }
}
</style>
