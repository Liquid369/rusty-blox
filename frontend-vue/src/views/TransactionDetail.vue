<template>
  <AppLayout>
    <div class="tx-detail-page">
      <div v-if="loading" class="skeleton" style="height: 400px;"></div>

      <div v-else-if="error" class="error-message">
        <h2>Transaction Not Found</h2>
        <p class="text-secondary">{{ error }}</p>
        <UiButton @click="$router.push('/')">Go to Dashboard</UiButton>
      </div>

      <div v-else-if="tx">
        <h1>Transaction Details</h1>
        <div class="badge-row">
          <span class="badge" :class="typeBadgeClass">{{ typeLabel }}</span>
          <span v-if="isMempool" class="badge badge-warning">Unconfirmed (Mempool)</span>
          <span v-else class="badge" :class="tx.confirmations >= 6 ? 'badge-success' : 'badge-warning'">
            {{ (tx.confirmations || 0).toLocaleString() }} Confirmation{{ tx.confirmations === 1 ? '' : 's' }}
          </span>
        </div>

        <UiCard class="mt-6">
          <template #header>
            <h2>Overview</h2>
          </template>

          <div class="detail-grid">
            <div class="detail-row">
              <span class="detail-label">Transaction ID</span>
              <span class="mono detail-value">{{ tx.txid }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Block</span>
              <router-link v-if="!isMempool && tx.blockHeight > 0" :to="`/block/${tx.blockHeight}`" class="detail-value detail-link">
                #{{ tx.blockHeight.toLocaleString() }}
              </router-link>
              <span v-else class="detail-value text-tertiary">Pending (in mempool)</span>
            </div>
            <div v-if="tx.blockHash" class="detail-row">
              <span class="detail-label">Block Hash</span>
              <router-link :to="`/block/${tx.blockHash}`" class="mono detail-value detail-link">
                {{ tx.blockHash }}
              </router-link>
            </div>
            <div class="detail-row">
              <span class="detail-label">Time</span>
              <span v-if="tx.blockTime" class="detail-value">{{ formatDate(tx.blockTime) }} ({{ formatTimeAgo(tx.blockTime) }})</span>
              <span v-else class="detail-value text-tertiary">Pending</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Total Value</span>
              <span class="mono detail-value amount-value">{{ formatSats(tx.value) }} PIV</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Input Value</span>
              <span class="mono detail-value">{{ formatSats(tx.valueIn) }} PIV</span>
            </div>
            <div v-if="showFees" class="detail-row">
              <span class="detail-label">Fee</span>
              <span class="mono detail-value fee-value">{{ formatSats(tx.fees) }} PIV</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Size</span>
              <span class="detail-value">{{ formatSize(tx.size) }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Version</span>
              <span class="detail-value">{{ tx.version }}</span>
            </div>
            <div v-if="tx.lockTime" class="detail-row">
              <span class="detail-label">Lock Time</span>
              <span class="detail-value">{{ tx.lockTime.toLocaleString() }}</span>
            </div>
          </div>
        </UiCard>

        <!-- Sapling Shielded Data -->
        <UiCard v-if="sapling" class="mt-6 sapling-card">
          <template #header>
            <div class="sapling-header">
              <h2>🛡️ Sapling Shielded Data</h2>
              <span class="badge badge-info">Private</span>
              <span v-if="shieldedTypeLabel" class="badge badge-info">{{ shieldedTypeLabel }}</span>
            </div>
          </template>

          <div class="detail-grid">
            <div v-if="sapling.shielded_spend_count !== undefined" class="detail-row">
              <span class="detail-label">Shielded Spends</span>
              <span class="detail-value">{{ sapling.shielded_spend_count }}</span>
            </div>
            <div v-if="sapling.shielded_output_count !== undefined" class="detail-row">
              <span class="detail-label">Shielded Outputs</span>
              <span class="detail-value">{{ sapling.shielded_output_count }}</span>
            </div>
            <div v-if="sapling.value_balance !== undefined" class="detail-row">
              <span class="detail-label">Value Balance</span>
              <span class="mono detail-value">
                {{ formatSats(sapling.value_balance) }} PIV
                <span class="text-tertiary">{{ valueBalanceExplanation }}</span>
              </span>
            </div>
            <div v-if="sapling.binding_sig" class="detail-row">
              <span class="detail-label">Binding Signature</span>
              <span class="mono detail-value">{{ truncateHash(sapling.binding_sig, 16) }}</span>
            </div>
          </div>

          <div v-if="sapling.spends?.length" class="shielded-list mt-6">
            <h3>Shielded Spend Details</h3>
            <div v-for="(spend, idx) in sapling.spends" :key="`spend-${idx}`" class="shielded-item">
              <span class="badge badge-info">Spend #{{ idx + 1 }}</span>
              <div class="shielded-fields">
                <div class="shielded-field"><span class="field-label">Nullifier</span><span class="mono">{{ truncateHash(spend.nullifier, 16) }}</span></div>
                <div class="shielded-field"><span class="field-label">Anchor</span><span class="mono">{{ truncateHash(spend.anchor, 16) }}</span></div>
                <div class="shielded-field"><span class="field-label">Value Commitment</span><span class="mono">{{ truncateHash(spend.cv, 16) }}</span></div>
              </div>
            </div>
          </div>

          <div v-if="sapling.outputs?.length" class="shielded-list mt-6">
            <h3>Shielded Output Details</h3>
            <div v-for="(output, idx) in sapling.outputs" :key="`shout-${idx}`" class="shielded-item">
              <span class="badge badge-success">Output #{{ idx + 1 }}</span>
              <div class="shielded-fields">
                <div class="shielded-field"><span class="field-label">Note Commitment</span><span class="mono">{{ truncateHash(output.cmu, 16) }}</span></div>
                <div class="shielded-field"><span class="field-label">Ephemeral Key</span><span class="mono">{{ truncateHash(output.ephemeral_key, 16) }}</span></div>
                <div class="shielded-field"><span class="field-label">Value Commitment</span><span class="mono">{{ truncateHash(output.cv, 16) }}</span></div>
              </div>
            </div>
          </div>

          <p class="sapling-note mt-4">
            Amounts, addresses, and memos within shielded transfers are encrypted with
            zero-knowledge proofs and are not visible on the blockchain.
          </p>
        </UiCard>

        <!-- Inputs -->
        <UiCard class="mt-6">
          <template #header>
            <div class="io-card-header">
              <h2>Inputs ({{ tx.vin?.length || 0 }})</h2>
              <span class="mono io-total">{{ formatSats(tx.valueIn) }} PIV</span>
            </div>
          </template>

          <div class="io-list">
            <div v-for="(input, idx) in tx.vin" :key="`in-${idx}`" class="io-item input-item">
              <div v-if="isCoinbaseInput(input)" class="coinbase-note">
                ⭐ {{ isCoinstake ? 'Coinstake' : 'Coinbase' }} — Newly Generated Coins
              </div>
              <div v-else class="io-content">
                <div class="io-top">
                  <span class="io-index">#{{ idx }}</span>
                  <span v-if="input.value !== undefined" class="mono io-amount">{{ formatSats(input.value) }} PIV</span>
                </div>
                <div v-if="input.addresses?.length" class="io-addresses">
                  <router-link
                    v-for="addr in input.addresses"
                    :key="addr"
                    :to="`/address/${addr}`"
                    class="mono io-address"
                  >
                    {{ addr }}
                  </router-link>
                </div>
                <span v-else class="text-tertiary">No address</span>
                <div v-if="input.txid" class="io-meta">
                  <span class="text-tertiary">Outpoint:</span>
                  <router-link :to="`/tx/${input.txid}`" class="mono io-outpoint">
                    {{ truncateHash(input.txid) }}:{{ input.vout }}
                  </router-link>
                </div>
              </div>
            </div>
          </div>
        </UiCard>

        <!-- Outputs -->
        <UiCard class="mt-6">
          <template #header>
            <div class="io-card-header">
              <h2>Outputs ({{ tx.vout?.length || 0 }})</h2>
              <span class="mono io-total">{{ formatSats(tx.value) }} PIV</span>
            </div>
          </template>

          <div class="io-list">
            <div v-for="output in tx.vout" :key="`out-${output.n}`" class="io-item output-item">
              <div class="io-content">
                <div class="io-top">
                  <span class="io-index">#{{ output.n }}</span>
                  <span class="mono io-amount">{{ formatSats(output.value) }} PIV</span>
                </div>
                <div v-if="output.addresses?.length" class="io-addresses">
                  <router-link
                    v-for="addr in output.addresses"
                    :key="addr"
                    :to="`/address/${addr}`"
                    class="mono io-address"
                  >
                    {{ addr }}
                  </router-link>
                </div>
                <span v-else class="text-tertiary">Nonstandard / no address</span>
                <div v-if="output.spent !== undefined && output.spent !== null" class="io-meta">
                  <span v-if="output.spent" class="badge badge-success">Spent</span>
                  <span v-else class="badge badge-info">Unspent</span>
                </div>
              </div>
            </div>
          </div>
        </UiCard>

        <!-- Raw Transaction -->
        <UiCard class="mt-6">
          <template #header>
            <button class="raw-toggle" @click="showRawHex = !showRawHex">
              <h2>Raw Transaction</h2>
              <span class="toggle-icon">{{ showRawHex ? '▼' : '▶' }}</span>
            </button>
          </template>

          <pre v-if="showRawHex" class="raw-hex mono">{{ tx.hex || 'Not available' }}</pre>
          <p v-else class="text-tertiary raw-hint">Click to expand the raw transaction hex.</p>
        </UiCard>
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { useRoute } from 'vue-router'
import { transactionService } from '@/services'
import AppLayout from '@/components/layout/AppLayout.vue'
import UiCard from '@/components/common/UiCard.vue'
import UiButton from '@/components/common/UiButton.vue'

const route = useRoute()

const loading = ref(true)
const error = ref('')
const tx = ref(null)
const showRawHex = ref(false)

// blockHeight of -1 means the tx is still in the mempool
const isMempool = computed(() => tx.value && tx.value.blockHeight < 0)

const sapling = computed(() => tx.value?.sapling || null)

const isCoinbaseInput = (input) => !input.txid && !input.addresses?.length

// PIVX coinstake: first input spends a prev output and first output is empty
const isCoinstake = computed(() => {
  const t = tx.value
  if (!t?.vout?.length) return false
  return Number(t.vout[0].value) === 0 && !!t.vin?.[0]?.txid
})

const isCoinbase = computed(() => {
  const vin = tx.value?.vin
  return !!(vin?.length && isCoinbaseInput(vin[0]) && !isCoinstake.value)
})

const isShielded = computed(() => {
  const s = sapling.value
  return !!(s && (s.shielded_spend_count > 0 || s.shielded_output_count > 0))
})

const typeLabel = computed(() => {
  if (isCoinstake.value) return 'Coinstake'
  if (isCoinbase.value) return 'Coinbase'
  if (isShielded.value) return 'Shielded'
  return 'Standard'
})

const typeBadgeClass = computed(() => {
  if (isCoinstake.value) return 'badge-success'
  if (isCoinbase.value) return 'badge-warning'
  if (isShielded.value) return 'badge-info'
  return 'badge-standard'
})

const shieldedTypeLabel = computed(() => {
  const type = sapling.value?.transaction_type
  if (type === 'shielding') return 'Shielding'
  if (type === 'unshielding') return 'Unshielding'
  if (type === 'shielded_transfer') return 'Shielded Transfer'
  return ''
})

const valueBalanceExplanation = computed(() => {
  const balance = Number(sapling.value?.value_balance)
  if (!balance) return '(Pure shielded transfer)'
  return balance < 0 ? '(Adding to shielded pool)' : '(Removing from shielded pool)'
})

// Coinbase/coinstake generate coins and don't pay fees
const showFees = computed(() => {
  if (isCoinbase.value || isCoinstake.value) return false
  return Number(tx.value?.fees) > 0
})

// Values from /v2/tx are string satoshis - divide by 1e8 only for display
const formatSats = (value) => {
  if (value === null || value === undefined || value === '') return '0.00000000'
  return (Number(value) / 100000000).toFixed(8)
}

const formatDate = (timestamp) => {
  return new Date(timestamp * 1000).toLocaleString()
}

const formatTimeAgo = (timestamp) => {
  const diff = Math.floor(Date.now() / 1000) - timestamp
  if (diff < 60) return `${diff}s ago`
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`
  return `${Math.floor(diff / 86400)}d ago`
}

const formatSize = (bytes) => {
  if (!bytes) return 'N/A'
  return `${bytes.toLocaleString()} bytes`
}

const truncateHash = (hash, len = 10) => {
  if (!hash) return ''
  if (hash.length <= len * 2) return hash
  return `${hash.slice(0, len)}...${hash.slice(-len)}`
}

const loadTransaction = async (txid) => {
  loading.value = true
  error.value = ''
  tx.value = null
  showRawHex.value = false

  try {
    tx.value = await transactionService.getTransaction(txid)
  } catch (err) {
    error.value = err.response?.data?.error?.message || 'The requested transaction could not be found.'
  } finally {
    loading.value = false
  }
}

watch(() => route.params.txid, (txid) => {
  if (txid) loadTransaction(txid)
}, { immediate: true })
</script>

<style scoped>
.tx-detail-page {
  animation: fadeIn 0.3s ease;
}

.badge-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex-wrap: wrap;
  margin-top: var(--space-3);
}

.badge-standard {
  background: var(--bg-tertiary);
  color: var(--text-secondary);
  border: 1px solid var(--border-subtle);
}

.detail-grid {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.detail-row {
  display: flex;
  justify-content: space-between;
  gap: var(--space-4);
  padding-bottom: var(--space-3);
  border-bottom: 1px solid var(--border-subtle);
}

.detail-row:last-child {
  border-bottom: none;
  padding-bottom: 0;
}

.detail-label {
  font-weight: var(--weight-bold);
  color: var(--text-secondary);
  flex-shrink: 0;
}

.detail-value {
  color: var(--text-primary);
  text-align: right;
  word-break: break-all;
}

.detail-link {
  color: var(--text-accent);
  text-decoration: none;
}

.detail-link:hover {
  text-decoration: underline;
}

.amount-value {
  font-weight: var(--weight-bold);
  color: var(--text-accent);
}

.fee-value {
  color: var(--warning);
}

.sapling-card {
  background: linear-gradient(135deg, rgba(102, 45, 145, 0.1) 0%, rgba(42, 27, 66, 0.2) 100%);
}

.sapling-header {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex-wrap: wrap;
}

.sapling-header h2 {
  margin: 0;
}

.shielded-list h3 {
  font-size: var(--text-lg);
  font-weight: var(--weight-bold);
  margin-bottom: var(--space-4);
}

.shielded-item {
  background: var(--bg-tertiary);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  padding: var(--space-4);
  margin-bottom: var(--space-3);
}

.shielded-fields {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  margin-top: var(--space-3);
}

.shielded-field {
  display: flex;
  gap: var(--space-3);
  font-size: var(--text-sm);
  word-break: break-all;
}

.field-label {
  color: var(--text-tertiary);
  font-size: var(--text-xs);
  min-width: 130px;
  flex-shrink: 0;
}

.sapling-note {
  font-size: var(--text-sm);
  color: var(--text-secondary);
  padding: var(--space-3);
  background: rgba(179, 255, 120, 0.05);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
}

.io-card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--space-4);
}

.io-card-header h2 {
  margin: 0;
}

.io-total {
  font-size: var(--text-sm);
  color: var(--text-tertiary);
  font-weight: var(--weight-semibold);
}

.io-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.io-item {
  padding: var(--space-4);
  border-radius: var(--radius-md);
  border-left: 3px solid;
}

.input-item {
  background: rgba(239, 68, 68, 0.08);
  border-left-color: var(--danger);
}

.output-item {
  background: rgba(16, 185, 129, 0.08);
  border-left-color: #10b981;
}

.io-content {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.io-top {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--space-4);
}

.io-index {
  font-size: var(--text-sm);
  color: var(--text-tertiary);
  font-weight: var(--weight-medium);
}

.io-amount {
  font-weight: var(--weight-bold);
  color: var(--text-accent);
}

.io-addresses {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.io-address {
  font-size: var(--text-sm);
  color: var(--text-accent);
  text-decoration: none;
  word-break: break-all;
}

.io-address:hover {
  text-decoration: underline;
}

.io-meta {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--text-xs);
}

.io-outpoint {
  color: var(--text-secondary);
  text-decoration: none;
}

.io-outpoint:hover {
  color: var(--text-accent);
}

.coinbase-note {
  color: var(--warning);
  font-weight: var(--weight-semibold);
}

.raw-toggle {
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

.raw-toggle h2 {
  margin: 0;
}

.toggle-icon {
  color: var(--text-tertiary);
}

.raw-hex {
  background: var(--bg-tertiary);
  padding: var(--space-4);
  border-radius: var(--radius-md);
  font-size: var(--text-sm);
  color: var(--text-secondary);
  word-break: break-all;
  white-space: pre-wrap;
  max-height: 400px;
  overflow-y: auto;
  margin: 0;
}

.raw-hint {
  font-size: var(--text-sm);
  margin: 0;
}

.error-message {
  text-align: center;
  padding: var(--space-16) var(--space-6);
}

.error-message p {
  margin-bottom: var(--space-6);
}

@media (max-width: 768px) {
  .detail-row {
    flex-direction: column;
  }

  .detail-value {
    text-align: left;
  }
}
</style>
