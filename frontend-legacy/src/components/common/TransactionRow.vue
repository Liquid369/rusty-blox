<template>
  <div
    class="transaction-row"
    :class="{ 'transaction-clickable': clickable }"
    @click="handleClick"
  >
    <!-- Transaction Type Badge -->
    <div class="tx-type">
      <Badge :variant="getTypeVariant(transaction.type)" size="sm">
        {{ getTypeLabel(transaction.type) }}
      </Badge>
    </div>

    <!-- Transaction ID -->
    <div class="tx-id">
      <span class="tx-label">TxID</span>
      <span class="tx-hash font-mono">
        {{ truncateHash(transaction.txid, 8, 8) }}
      </span>
      <CopyButton v-if="showCopy" :text="transaction.txid" class="copy-btn" />
    </div>

    <!-- Transaction Info -->
    <div class="tx-info">
      <div class="tx-detail">
        <span class="tx-detail-label">Inputs</span>
        <span class="tx-detail-value">{{ transaction.vin?.length || 0 }}</span>
      </div>
      <div class="tx-detail">
        <span class="tx-detail-label">Outputs</span>
        <span class="tx-detail-value">{{ transaction.vout?.length || 0 }}</span>
      </div>
      <!-- Net delta for the viewed address (signed + colored) -->
      <div v-if="hasNet" class="tx-detail">
        <span class="tx-detail-label">Net</span>
        <span
          class="tx-detail-value tx-amount"
          :class="netSats >= 0 ? 'tx-net-positive' : 'tx-net-negative'"
        >
          {{ netSats >= 0 ? '+' : '-' }}{{ formatPIV(Math.abs(netSats)) }} PIV
        </span>
      </div>
      <!-- Gross tx value when no viewed address is provided (dashboard/block/etc.) -->
      <div v-else-if="transaction.value !== undefined" class="tx-detail">
        <span class="tx-detail-label">Amount</span>
        <span class="tx-detail-value tx-amount">
          {{ formatPIV(transaction.value) }} PIV
        </span>
      </div>
    </div>

    <!-- Timestamp -->
    <div v-if="transaction.blockTime" class="tx-time">
      <span class="tx-time-ago">{{ formatTimeAgo(transaction.blockTime) }}</span>
      <span class="tx-time-full">{{ formatDate(transaction.blockTime) }}</span>
    </div>

    <!-- Confirmations -->
    <div v-if="transaction.confirmations !== undefined" class="tx-confirmations">
      <Badge
        :variant="transaction.confirmations >= 6 ? 'success' : 'warning'"
        size="sm"
      >
        {{ transaction.confirmations }} conf
      </Badge>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import { formatPIV, formatTimeAgo, formatDate, truncateHash } from '@/utils/formatters'
import { getTransactionTypeLabel, getTransactionTypeBadgeVariant, toSats } from '@/utils/transactionHelpers'
import Badge from './Badge.vue'
import CopyButton from './CopyButton.vue'

const emit = defineEmits(['click'])

const props = defineProps({
  transaction: {
    type: Object,
    required: true
  },
  clickable: {
    type: Boolean,
    default: true
  },
  showCopy: {
    type: Boolean,
    default: true
  },
  // Address(es) the page is viewing. A single string or an array of strings
  // (e.g. an xpub's derived addresses). When provided, the row renders a
  // signed, colored NET delta for that address set instead of the gross tx
  // value. When absent, behavior is unchanged (gross value).
  viewedAddresses: {
    type: [String, Array],
    default: null
  }
})

// Set of viewed addresses (empty when none provided).
const viewedSet = computed(() => {
  const v = props.viewedAddresses
  if (!v) return null
  const list = Array.isArray(v) ? v : [v]
  const set = new Set(list.filter(a => typeof a === 'string' && a.length > 0))
  return set.size > 0 ? set : null
})

// Collect every address attached to a vin/vout entry. Outputs may expose the
// address list directly (`addresses`) or nested under `scriptPubKey`.
const entryAddresses = (entry) => {
  if (!entry) return []
  if (Array.isArray(entry.addresses)) return entry.addresses
  if (Array.isArray(entry.scriptPubKey?.addresses)) return entry.scriptPubKey.addresses
  return []
}

const isViewed = (entry, set) =>
  entryAddresses(entry).some(a => set.has(a))

// Net delta in SATOSHIS for the viewed address set:
//   net = Σ vout.value (addr ∈ viewed) − Σ vin.value (addr ∈ viewed)
// vin/vout .value are satoshi strings (see InputOutputTable/formatPIV), so we
// keep the result in satoshis and hand it straight to formatPIV (which divides
// by 1e8 exactly once). No extra scaling here.
const netSats = computed(() => {
  const set = viewedSet.value
  if (!set) return 0
  const tx = props.transaction
  let received = 0
  let spent = 0
  for (const out of tx.vout || []) {
    if (isViewed(out, set)) received += toSats(out.value)
  }
  for (const inp of tx.vin || []) {
    if (isViewed(inp, set)) spent += toSats(inp.value)
  }
  return received - spent
})

// Only render the net column when a viewed address set is actually provided.
const hasNet = computed(() => viewedSet.value !== null)

const handleClick = () => {
  if (props.clickable) {
    emit('click', props.transaction)
  }
}

const getTypeVariant = (type) => {
  return getTransactionTypeBadgeVariant(type)
}

const getTypeLabel = (type) => {
  return getTransactionTypeLabel(type)
}
</script>

<style scoped>
.transaction-row {
  display: grid;
  grid-template-columns: auto 1fr auto auto;
  gap: var(--space-4);
  padding: var(--space-4);
  background: var(--glass-bg-subtle);
  border: 1px solid var(--border-secondary);
  border-radius: var(--radius-md);
  align-items: center;
  transition:
    background-color var(--transition-fast),
    border-color var(--transition-fast),
    box-shadow var(--transition-fast),
    transform var(--transition-fast);
}

.transaction-clickable {
  cursor: pointer;
}

.transaction-clickable:hover {
  background: rgba(var(--rgb-purple-mid), 0.4);
  border-color: var(--glass-border-hover);
  box-shadow: var(--shadow-sm), var(--glow-purple);
  transform: translateX(4px);
}

.tx-type {
  display: flex;
  align-items: center;
}

.tx-id {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  min-width: 0;
}

.tx-label {
  font-size: var(--text-xs);
  color: var(--text-tertiary);
  text-transform: uppercase;
  font-weight: var(--weight-bold);
  letter-spacing: 0.5px;
}

.tx-hash {
  font-size: var(--text-sm);
  color: var(--text-primary);
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.copy-btn {
  padding: 2px 4px;
  font-size: 10px;
}

.tx-info {
  display: flex;
  gap: var(--space-4);
  align-items: center;
}

.tx-detail {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.tx-detail-label {
  font-size: var(--text-xs);
  color: var(--text-tertiary);
  font-weight: var(--weight-medium);
}

.tx-detail-value {
  font-size: var(--text-sm);
  color: var(--text-secondary);
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
}

.tx-amount {
  color: var(--pivx-accent);
  font-weight: var(--weight-bold);
}

/* Signed net delta for the viewed address: green gain, red loss. */
.tx-net-positive {
  color: var(--success);
}

.tx-net-negative {
  color: var(--danger);
}

.tx-time {
  display: flex;
  flex-direction: column;
  gap: 2px;
  text-align: right;
}

.tx-time-ago {
  font-size: var(--text-sm);
  color: var(--text-primary);
  font-weight: var(--weight-medium);
}

.tx-time-full {
  font-size: var(--text-xs);
  color: var(--text-tertiary);
}

.tx-confirmations {
  display: flex;
  align-items: center;
}

@media (max-width: 1024px) {
  .transaction-row {
    grid-template-columns: 1fr;
    gap: var(--space-3);
  }

  .tx-info {
    flex-wrap: wrap;
    gap: var(--space-3);
  }

  .tx-time {
    text-align: left;
  }
}

@media (max-width: 768px) {
  .transaction-row {
    padding: var(--space-3);
  }

  .tx-info {
    gap: var(--space-2);
  }
}
</style>
