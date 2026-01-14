<template>
  <AppLayout>
    <div class="transaction-detail-page">
      <!-- Loading State -->
      <div v-if="loading" class="loading-container">
        <LoadingSpinner size="lg" text="Loading transaction..." />
      </div>

      <!-- Error State -->
      <div v-else-if="error" class="error-container">
        <EmptyState
          icon="⚠️"
          title="Transaction Not Found"
          :message="error"
        >
          <template #action>
            <Button @click="$router.push('/')">Go to Dashboard</Button>
          </template>
        </EmptyState>
      </div>

      <!-- Transaction Details -->
      <div v-else-if="transaction">
        <!-- Header -->
        <div class="page-header">
          <div class="header-content">
            <h1>Transaction Details</h1>
            <Badge :variant="transactionTypeBadgeVariant">
              {{ transactionTypeLabel }}
            </Badge>
          </div>
        </div>

        <!-- Transaction Info Card -->
        <Card class="transaction-info-card">
          <template #header>Basic Information</template>

          <div class="info-grid">
            <InfoRow label="Transaction ID" icon="🆔">
              <HashDisplay :hash="transaction.txid" :truncate="false" show-copy />
            </InfoRow>

            <InfoRow label="Block" icon="📦">
              <div class="block-info">
                <HashDisplay
                  v-if="transaction.blockHash"
                  :hash="transaction.blockHash"
                  :truncate="true"
                  show-copy
                  :link-to="`/block/${transaction.blockHeight}`"
                />
                <span v-else class="unconfirmed">Unconfirmed</span>
                <span v-if="transaction.blockHeight" class="block-height">
                  Height: {{ formatNumber(transaction.blockHeight) }}
                </span>
              </div>
            </InfoRow>

            <InfoRow label="Confirmations" icon="✅">
              <div class="confirmation-display">
                <Badge :variant="confirmationBadgeVariant">
                  {{ confirmations }} confirmation{{ confirmations !== 1 ? 's' : '' }}
                </Badge>
                <Badge v-if="isSyncLagging" variant="warning" class="sync-warning">
                  ⚠️ Node syncing ({{ syncLag }} blocks behind)
                </Badge>
              </div>
            </InfoRow>

            <InfoRow label="Timestamp" icon="🕐">
              <div class="timestamp-group" v-if="transaction.blockTime">
                <span>{{ formatDate(transaction.blockTime) }}</span>
                <span class="time-ago">{{ formatTimeAgo(transaction.blockTime) }}</span>
              </div>
              <span v-else class="unconfirmed">Pending</span>
            </InfoRow>

            <InfoRow label="Amount" icon="💰">
              <span class="amount-value">{{ formatPIV(transaction.value) }} PIV</span>
            </InfoRow>

            <InfoRow v-if="shouldShowFees" label="Transaction Fee" icon="⚙️">
              <span class="fee-value">{{ formatPIV(transaction.fees) }} PIV</span>
            </InfoRow>

            <InfoRow label="Size" icon="💾">
              {{ formatBytes(transaction.size || 0) }}
            </InfoRow>

            <InfoRow label="Version" icon="🔢">
              {{ transaction.version }}
            </InfoRow>

            <InfoRow v-if="transaction.locktime" label="Lock Time" icon="🔒">
              {{ formatNumber(transaction.locktime) }}
            </InfoRow>
          </div>
        </Card>

        <!-- Sapling Info (if applicable) -->
        <Card v-if="isShieldedTransaction" class="sapling-card">
          <template #header>
            <div class="card-header-content">
              <span>🛡️ Sapling Shielded Transaction</span>
              <Badge variant="accent">Private</Badge>
              <Badge v-if="transaction.sapling?.transaction_type" :variant="shieldedTypeBadgeVariant">
                {{ shieldedTypeLabel }}
              </Badge>
            </div>
          </template>

          <div class="info-grid">
            <InfoRow v-if="transaction.sapling?.shielded_spend_count !== undefined" label="Shielded Spends" icon="🔒">
              <div class="shielded-count">
                <span class="count-number">{{ transaction.sapling.shielded_spend_count }}</span>
                <span class="count-label">input{{ transaction.sapling.shielded_spend_count !== 1 ? 's' : '' }}</span>
                <span v-if="transaction.sapling.shielded_spend_count > 0" class="info-detail">(consuming shielded notes)</span>
              </div>
            </InfoRow>

            <InfoRow v-if="transaction.sapling?.shielded_output_count !== undefined" label="Shielded Outputs" icon="🔐">
              <div class="shielded-count">
                <span class="count-number">{{ transaction.sapling.shielded_output_count }}</span>
                <span class="count-label">output{{ transaction.sapling.shielded_output_count !== 1 ? 's' : '' }}</span>
                <span v-if="transaction.sapling.shielded_output_count > 0" class="info-detail">(creating shielded notes)</span>
              </div>
            </InfoRow>

            <InfoRow v-if="formatPIV(transaction.sapling?.value_balance)" label="Value Balance" icon="⚖️">
              <div class="value-balance">
                <span class="balance-amount" :class="valueBalanceClass">{{ formatPIV(transaction.sapling.value_balance) }} PIV</span>
                <span class="balance-explanation">{{ valueBalanceExplanation }}</span>
              </div>
            </InfoRow>

            <InfoRow v-if="transaction.sapling?.binding_sig" label="Binding Signature" icon="🔐">
              <HashDisplay :hash="transaction.sapling.binding_sig" :truncate="true" show-copy />
            </InfoRow>
          </div>

          <!-- Shielded Spend Details -->
          <div v-if="transaction.sapling?.spends?.length > 0" class="shielded-details">
            <h3 class="details-title">🔒 Shielded Spend Details</h3>
            <div class="spend-list">
              <div v-for="(spend, idx) in transaction.sapling.spends" :key="idx" class="spend-item">
                <div class="spend-header">
                  <Badge variant="info">Spend #{{ idx + 1 }}</Badge>
                </div>
                <div class="spend-fields">
                  <div class="field-row">
                    <span class="field-label">Nullifier:</span>
                    <HashDisplay :hash="spend.nullifier" :truncate="true" show-copy />
                  </div>
                  <div class="field-row">
                    <span class="field-label">Anchor:</span>
                    <HashDisplay :hash="spend.anchor" :truncate="true" show-copy />
                  </div>
                  <div class="field-row">
                    <span class="field-label">Value Commitment:</span>
                    <HashDisplay :hash="spend.cv" :truncate="true" show-copy />
                  </div>
                </div>
              </div>
            </div>
          </div>

          <!-- Shielded Output Details -->
          <div v-if="transaction.sapling?.outputs?.length > 0" class="shielded-details">
            <h3 class="details-title">🔐 Shielded Output Details</h3>
            <div class="output-list">
              <div v-for="(output, idx) in transaction.sapling.outputs" :key="idx" class="output-item">
                <div class="output-header">
                  <Badge variant="success">Output #{{ idx + 1 }}</Badge>
                </div>
                <div class="output-fields">
                  <div class="field-row">
                    <span class="field-label">Note Commitment:</span>
                    <HashDisplay :hash="output.cmu" :truncate="true" show-copy />
                  </div>
                  <div class="field-row">
                    <span class="field-label">Ephemeral Key:</span>
                    <HashDisplay :hash="output.ephemeral_key" :truncate="true" show-copy />
                  </div>
                  <div class="field-row">
                    <span class="field-label">Value Commitment:</span>
                    <HashDisplay :hash="output.cv" :truncate="true" show-copy />
                  </div>
                  <div class="field-row">
                    <span class="field-label">Encrypted Ciphertext:</span>
                    <span class="ciphertext-info">580 bytes (contains encrypted amount, memo, and recipient info)</span>
                  </div>
                </div>
              </div>
            </div>
          </div>

          <div class="sapling-note">
            <div class="note-icon">ℹ️</div>
            <div class="note-content">
              <p class="note-title">Privacy Information</p>
              <p>This transaction contains Sapling shielded components. The amounts, addresses, and memos within shielded transfers are encrypted using zero-knowledge proofs and are not visible on the blockchain. Only the transaction participants with the correct viewing keys can decrypt this information.</p>
              <ul class="privacy-features">
                <li><strong>Shielded Amounts:</strong> Values are cryptographically hidden using Pedersen commitments</li>
                <li><strong>Private Addresses:</strong> Recipient addresses are encrypted and unlinkable</li>
                <li><strong>Zero-Knowledge Proofs:</strong> Transactions are verified without revealing transaction details</li>
                <li v-if="transaction.sapling?.transaction_type === 'shielding'"><strong>Shielding:</strong> Moving funds from transparent to shielded pool for privacy</li>
                <li v-if="transaction.sapling?.transaction_type === 'unshielding'"><strong>Unshielding:</strong> Moving funds from shielded to transparent pool</li>
              </ul>
            </div>
          </div>
        </Card>

        <!-- Inputs and Outputs Table -->
        <div class="io-section">
          <h2 class="section-title">Inputs & Outputs</h2>
          <InputOutputTable
            :inputs="transaction.vin"
            :outputs="transaction.vout"
            :fees="transaction.fees"
          />
        </div>

        <!-- Raw Transaction (Collapsible) -->
        <Card class="raw-tx-card">
          <template #header>
            <button class="raw-tx-toggle" @click="showRawTx = !showRawTx">
              <span>Raw Transaction</span>
              <span class="toggle-icon">{{ showRawTx ? '▼' : '▶' }}</span>
            </button>
          </template>

          <div v-if="showRawTx" class="raw-tx-content">
            <pre class="raw-tx-hex">{{ transaction.hex || 'Not available' }}</pre>
            <CopyButton v-if="transaction.hex" :text="transaction.hex" />
          </div>
        </Card>
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useChainStore } from '@/stores/chainStore'
import { transactionService } from '@/services/transactionService'
import { detectTransactionType, getTransactionTypeLabel, getTransactionTypeBadgeVariant } from '@/utils/transactionHelpers'
import { formatNumber, formatDate, formatTimeAgo, formatBytes, formatPIV } from '@/utils/formatters'
import AppLayout from '@/components/layout/AppLayout.vue'
import Card from '@/components/common/Card.vue'
import Badge from '@/components/common/Badge.vue'
import Button from '@/components/common/Button.vue'
import InfoRow from '@/components/common/InfoRow.vue'
import HashDisplay from '@/components/common/HashDisplay.vue'
import LoadingSpinner from '@/components/common/LoadingSpinner.vue'
import EmptyState from '@/components/common/EmptyState.vue'
import InputOutputTable from '@/components/common/InputOutputTable.vue'
import CopyButton from '@/components/common/CopyButton.vue'

const route = useRoute()
const router = useRouter()
const chainStore = useChainStore()

const transaction = ref(null)
const loading = ref(false)
const error = ref('')
const showRawTx = ref(false)

const transactionType = computed(() => {
  return transaction.value ? detectTransactionType(transaction.value) : 'regular'
})

const transactionTypeLabel = computed(() => {
  return getTransactionTypeLabel(transactionType.value)
})

const transactionTypeBadgeVariant = computed(() => {
  return getTransactionTypeBadgeVariant(transactionType.value)
})

// Fees are not applicable for coinbase/coinstake (they generate coins, don't pay fees)
const shouldShowFees = computed(() => {
  if (!transaction.value?.fees) return false
  if (transaction.value.fees === '0' || transaction.value.fees === '0.00000000') return false
  
  // Hide for coinbase and coinstake types
  return transactionType.value !== 'coinbase' && transactionType.value !== 'coinstake'
})

const confirmations = computed(() => {
  // Use networkHeight for actual chain depth, not syncHeight (local index)
  if (!transaction.value?.blockHeight || !chainStore.networkHeight) return 0
  
  // Handle special heights: -1 (mempool/unconfirmed), -2 (orphaned)
  if (transaction.value.blockHeight <= 0) return 0
  
  return Math.max(0, chainStore.networkHeight - transaction.value.blockHeight + 1)
})

// Detect when local index lags behind network
const syncLag = computed(() => {
  if (!chainStore.networkHeight || !chainStore.syncHeight) return 0
  return Math.max(0, chainStore.networkHeight - chainStore.syncHeight)
})

// Show warning if node is catching up (>10 blocks behind)
const isSyncLagging = computed(() => syncLag.value > 10)

const confirmationBadgeVariant = computed(() => {
  const conf = confirmations.value
  if (conf === 0) return 'warning'
  if (conf < 6) return 'info'
  if (isSyncLagging.value) return 'warning' // Show warning during sync lag
  return 'success'
})

const isShieldedTransaction = computed(() => {
  return transactionType.value === 'shielded' || 
         transaction.value?.sapling?.shielded_spend_count > 0 ||
         transaction.value?.sapling?.shielded_output_count > 0
})

const shieldedTypeLabel = computed(() => {
  const type = transaction.value?.sapling?.transaction_type
  if (type === 'shielding') return '🛡️ Shielding'
  if (type === 'unshielding') return '🔓 Unshielding'
  if (type === 'shielded_transfer') return '🔐 Shielded Transfer'
  return 'Unknown'
})

const shieldedTypeBadgeVariant = computed(() => {
  const type = transaction.value?.sapling?.transaction_type
  if (type === 'shielding') return 'info'
  if (type === 'unshielding') return 'warning'
  if (type === 'shielded_transfer') return 'success'
  return 'default'
})

const valueBalanceClass = computed(() => {
  const balance = transaction.value?.sapling?.value_balance_sat
  if (!balance) return ''
  return balance < 0 ? 'balance-negative' : balance > 0 ? 'balance-positive' : 'balance-zero'
})

const valueBalanceExplanation = computed(() => {
  const balance = transaction.value?.sapling?.value_balance_sat
  if (!balance) return ''
  if (balance < 0) return '(Adding to shielded pool)'
  if (balance > 0) return '(Removing from shielded pool)'
  return '(Pure shielded transfer)'
})

const fetchTransaction = async (txid) => {
  loading.value = true
  error.value = ''
  transaction.value = null

  try {
    const txData = await transactionService.getTransaction(txid)
    transaction.value = txData
  } catch (err) {
    console.error('Failed to fetch transaction:', err)
    error.value = err.message || 'Failed to load transaction'
  } finally {
    loading.value = false
  }
}

watch(() => route.params.txid, (newTxid) => {
  if (newTxid) {
    fetchTransaction(newTxid)
    chainStore.fetchChainState()
  }
}, { immediate: true })

// Watch for reorg detection and refetch transaction
watch(() => chainStore.reorgDetected, (detected) => {
  if (detected && route.params.txid) {
    console.log('🔄 Reorg detected - refetching transaction data')
    fetchTransaction(route.params.txid)
  }
})
</script>

<style scoped>
.transaction-detail-page {
  padding: var(--space-6);
  max-width: 1400px;
  margin: 0 auto;
}

.page-header {
  margin-bottom: var(--space-6);
}

.header-content {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
}

.transaction-info-card,
.sapling-card,
.io-section,
.raw-tx-card {
  margin-bottom: var(--space-6);
}

.info-grid {
  display: grid;
  gap: var(--space-4);
}

.block-info {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.block-height {
  font-size: var(--text-sm);
  color: var(--text-secondary);
}

.timestamp-group {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.time-ago {
  font-size: var(--text-sm);
  color: var(--text-tertiary);
}

.unconfirmed {
  color: var(--text-warning);
  font-weight: 600;
}

.amount-value {
  font-size: var(--text-xl);
  font-weight: 700;
  color: var(--text-accent);
  font-family: var(--font-mono);
}

.fee-value {
  font-family: var(--font-mono);
  color: var(--text-secondary);
}

.card-header-content {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex-wrap: wrap;
}

.sapling-card {
  background: linear-gradient(135deg, rgba(102, 45, 145, 0.1) 0%, rgba(42, 27, 66, 0.3) 100%);
  border: 2px solid rgba(89, 252, 179, 0.2);
}

.shielded-count {
  display: flex;
  align-items: baseline;
  gap: var(--space-2);
  flex-wrap: wrap;
}

.count-number {
  font-size: var(--text-2xl);
  font-weight: var(--weight-bold);
  color: var(--pivx-accent);
  font-family: var(--font-mono);
}

.count-label {
  font-size: var(--text-base);
  color: var(--text-secondary);
  font-weight: var(--weight-medium);
}

.info-detail {
  font-size: var(--text-sm);
  color: var(--text-tertiary);
  font-style: italic;
}

.value-balance {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.balance-amount {
  font-size: var(--text-xl);
  font-weight: var(--weight-bold);
  font-family: var(--font-mono);
}

.balance-negative {
  color: var(--pivx-accent);
  text-shadow: 0 0 10px rgba(89, 252, 179, 0.3);
}

.balance-positive {
  color: var(--warning);
}

.balance-zero {
  color: var(--text-secondary);
}

.balance-explanation {
  font-size: var(--text-sm);
  color: var(--text-tertiary);
  font-style: italic;
}

.shielded-details {
  margin-top: var(--space-6);
  padding-top: var(--space-6);
  border-top: 2px solid var(--border-subtle);
}

.details-title {
  font-size: var(--text-lg);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
  margin-bottom: var(--space-4);
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.spend-list,
.output-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.spend-item,
.output-item {
  background: var(--bg-tertiary);
  border: 1px solid var(--border-secondary);
  border-radius: var(--radius-md);
  padding: var(--space-4);
  transition: all var(--transition-base);
}

.spend-item:hover,
.output-item:hover {
  border-color: var(--pivx-accent);
  box-shadow: 0 0 12px rgba(89, 252, 179, 0.2);
}

.spend-header,
.output-header {
  margin-bottom: var(--space-3);
}

.spend-fields,
.output-fields {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.field-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-2);
  background: var(--bg-quaternary);
  border-radius: var(--radius-sm);
}

.field-label {
  font-size: var(--text-sm);
  font-weight: var(--weight-bold);
  color: var(--text-secondary);
  min-width: 140px;
  flex-shrink: 0;
}

.ciphertext-info {
  font-size: var(--text-sm);
  color: var(--text-tertiary);
  font-style: italic;
}

.sapling-note {
  margin-top: var(--space-6);
  padding: var(--space-4);
  background: rgba(89, 252, 179, 0.05);
  border: 1px solid rgba(89, 252, 179, 0.2);
  border-radius: var(--radius-md);
  display: flex;
  gap: var(--space-3);
}

.note-icon {
  font-size: var(--text-2xl);
  flex-shrink: 0;
}

.note-content {
  flex: 1;
}

.note-title {
  font-size: var(--text-base);
  font-weight: var(--weight-bold);
  color: var(--pivx-accent);
  margin-bottom: var(--space-2);
}

.note-content p {
  font-size: var(--text-sm);
  color: var(--text-secondary);
  line-height: 1.6;
  margin-bottom: var(--space-3);
}

.privacy-features {
  list-style: none;
  padding: 0;
  margin: var(--space-3) 0 0 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.privacy-features li {
  font-size: var(--text-sm);
  color: var(--text-secondary);
  padding-left: var(--space-4);
  position: relative;
}

.privacy-features li::before {
  content: '✓';
  position: absolute;
  left: 0;
  color: var(--pivx-accent);
  font-weight: bold;
}

.privacy-features li strong {
  color: var(--text-primary);
  font-weight: var(--weight-bold);
}

.section-title {
  margin-bottom: var(--space-4);
  color: var(--text-primary);
}

.raw-tx-toggle {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  background: none;
  border: none;
  color: inherit;
  font: inherit;
  cursor: pointer;
  padding: 0;
}

.toggle-icon {
  color: var(--text-tertiary);
  transition: transform 0.2s;
}

.raw-tx-content {
  position: relative;
  margin-top: var(--space-4);
}

.confirmation-display {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-wrap: wrap;
}

.sync-warning {
  font-size: var(--text-xs);
}

.raw-tx-hex {
  background: var(--bg-tertiary);
  padding: var(--space-4);
  border-radius: var(--radius-md);
  font-family: var(--font-mono);
  font-size: var(--text-sm);
  color: var(--text-secondary);
  overflow-x: auto;
  word-break: break-all;
  white-space: pre-wrap;
  max-height: 400px;
  overflow-y: auto;
}

.loading-container,
.error-container {
  min-height: 400px;
  display: flex;
  align-items: center;
  justify-content: center;
}

@media (max-width: 768px) {
  .transaction-detail-page {
    padding: var(--space-4);
  }

  .header-content {
    flex-direction: column;
    align-items: flex-start;
  }

  .amount-value {
    font-size: var(--text-lg);
  }
}
</style>
