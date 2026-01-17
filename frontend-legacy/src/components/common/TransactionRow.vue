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
      <div v-if="transaction.value !== undefined" class="tx-detail">
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
import { formatPIV, formatTimeAgo, formatDate, truncateHash } from '@/utils/formatters'
import { getTransactionTypeLabel, getTransactionTypeBadgeVariant } from '@/utils/transactionHelpers'
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
  }
})

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
  background: var(--bg-secondary);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  align-items: center;
  transition: all var(--transition-fast);
}

.transaction-clickable {
  cursor: pointer;
}

.transaction-clickable:hover {
  background: var(--bg-tertiary);
  border-color: var(--border-accent);
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
}

.tx-amount {
  color: var(--pivx-accent);
  font-weight: var(--weight-bold);
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
