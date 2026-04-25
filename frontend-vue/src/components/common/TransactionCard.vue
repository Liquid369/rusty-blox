<template>
  <div class="tx-card-wrapper" @click="$emit('click', tx.txid)">
    <div class="tx-card-header">
      <div class="tx-type-section">
        <div class="tx-type-badge" :class="`tx-type-${tx.tx_type}`">
          {{ formatTxType(tx.tx_type) }}
        </div>
        <span class="tx-id-label">{{ truncateHash(tx.txid) }}</span>
      </div>
      <div class="tx-summary">
        <div class="summary-item">
          <span class="summary-label">Value</span>
          <span class="summary-value">{{ formatPIV(tx.value_out) }} PIV</span>
        </div>
        <div class="summary-item" v-if="tx.fees > 0">
          <span class="summary-label">Fee</span>
          <span class="summary-value fee">{{ formatPIV(tx.fees) }} PIV</span>
        </div>
      </div>
    </div>

    <!-- Inputs Section -->
    <div class="tx-section">
      <div class="section-title">
        📥 Inputs ({{ tx.vin?.length || 0 }})
        <span class="section-total">Total: {{ formatPIV(tx.value_in) }} PIV</span>
      </div>
      <div class="io-list">
        <div v-for="(input, idx) in tx.vin" :key="`in-${idx}`" class="io-item input-item">
          <div v-if="input.coinbase" class="coinbase-badge">
            ⭐ Coinbase (Newly Generated Coins)
            <code class="coinbase-data">{{ input.coinbase }}</code>
          </div>
          <div v-else class="regular-io">
            <div class="io-header">
              <span class="io-index">Input #{{ idx }}</span>
              <span v-if="input.value" class="io-amount">{{ formatPIV(input.value) }} PIV</span>
            </div>
            <div v-if="input.type === 'coldstake' && input.addresses && input.addresses.length === 2" class="coldstake-box">
              <div class="coldstake-title">❄️ Cold Staking Contract</div>
              <div class="coldstake-address">
                <span class="address-label">Staker:</span>
                <code>{{ input.addresses[0] }}</code>
              </div>
              <div class="coldstake-address">
                <span class="address-label">Owner:</span>
                <code>{{ input.addresses[1] }}</code>
              </div>
            </div>
            <div v-else class="io-address">
              <span class="address-label">From:</span>
              <code>{{ getAddress(input) }}</code>
            </div>
            <div class="io-meta">
              <span class="meta-label">Prev TX:</span>
              <code class="meta-value">{{ input.txid ? `${truncateHash(input.txid)}:${input.vout}` : 'N/A' }}</code>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- Outputs Section -->
    <div class="tx-section">
      <div class="section-title">
        📤 Outputs ({{ tx.vout?.length || 0 }})
        <span class="section-total">Total: {{ formatPIV(tx.value_out) }} PIV</span>
      </div>
      <div class="io-list">
        <div v-for="(output, idx) in tx.vout" :key="`out-${idx}`" class="io-item output-item">
          <div class="regular-io">
            <div class="io-header">
              <span class="io-index">Output #{{ output.n }}</span>
              <span class="io-amount">{{ formatPIV(output.value) }} PIV</span>
            </div>
            <div v-if="output.type === 'coldstake' && output.addresses && output.addresses.length === 2" class="coldstake-box">
              <div class="coldstake-title">❄️ Cold Staking Contract</div>
              <div class="coldstake-address">
                <span class="address-label">Staker:</span>
                <code>{{ output.addresses[0] }}</code>
              </div>
              <div class="coldstake-address">
                <span class="address-label">Owner:</span>
                <code>{{ output.addresses[1] }}</code>
              </div>
            </div>
            <div v-else-if="output.addresses && output.addresses.length > 0" class="io-address">
              <span class="address-label">To:</span>
              <code>{{ output.addresses[0] }}</code>
            </div>
            <div v-if="output.type" class="io-meta">
              <span class="meta-label">Type:</span>
              <span class="meta-value">{{ output.type }}</span>
              <span v-if="output.spent" class="spent-badge">✓ Spent</span>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- Reward Banner -->
    <div v-if="tx.reward > 0" class="reward-banner">
      🎉 Staking Reward: <strong>{{ tx.reward }} PIV</strong>
    </div>
  </div>
</template>

<script setup>
const props = defineProps({
  tx: {
    type: Object,
    required: true
  }
})

const emit = defineEmits(['click'])

const formatTxType = (type) => {
  const types = {
    'coinbase': 'Coinbase',
    'coinstake': 'Coinstake',
    'standard': 'Standard',
    'coldstake': 'Cold Stake',
    'shield': 'Shield'
  }
  return types[type] || type
}

const formatPIV = (value) => {
  if (!value) return '0.00'
  return Number(value).toFixed(8)
}

const truncateHash = (hash) => {
  if (!hash) return ''
  if (hash.length <= 20) return hash
  return `${hash.slice(0, 10)}...${hash.slice(-10)}`
}

const getAddress = (io) => {
  if (io.addresses && io.addresses[0]) return io.addresses[0]
  if (io.address) return io.address
  return 'N/A'
}
</script>

<style scoped>
.tx-card-wrapper {
  background: var(--bg-secondary);
  border: 2px solid var(--border-primary);
  border-radius: var(--radius-lg);
  padding: var(--space-6);
  cursor: pointer;
  transition: all var(--transition-base);
}

.tx-card-wrapper:hover {
  transform: translateY(-2px);
  border-color: var(--text-accent);
  box-shadow: 0 8px 24px rgba(89, 252, 179, 0.1);
}

.tx-card-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  padding-bottom: var(--space-4);
  margin-bottom: var(--space-4);
  border-bottom: 2px solid var(--border-primary);
  gap: var(--space-4);
}

.tx-type-section {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex: 1;
  min-width: 0;
}

.tx-type-badge {
  padding: var(--space-1) var(--space-3);
  border-radius: var(--radius-sm);
  font-size: var(--text-xs);
  font-weight: var(--weight-extrabold);
  text-transform: uppercase;
  letter-spacing: 0.05em;
  white-space: nowrap;
}

.tx-type-coinbase {
  background: rgba(251, 191, 36, 0.2);
  color: #fbbf24;
  border: 1px solid rgba(251, 191, 36, 0.4);
}

.tx-type-coinstake {
  background: rgba(102, 45, 145, 0.2);
  color: var(--text-accent);
  border: 1px solid rgba(102, 45, 145, 0.4);
}

.tx-type-standard {
  background: var(--bg-tertiary);
  color: var(--text-secondary);
  border: 1px solid var(--border-subtle);
}

.tx-type-coldstake {
  background: rgba(89, 252, 179, 0.15);
  color: var(--text-accent);
  border: 1px solid rgba(89, 252, 179, 0.3);
}

.tx-id-label {
  font-family: var(--font-mono);
  font-size: var(--text-sm);
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
}

.tx-summary {
  display: flex;
  gap: var(--space-6);
}

.summary-item {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: var(--space-1);
}

.summary-label {
  font-size: var(--text-xs);
  color: var(--text-tertiary);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.summary-value {
  font-size: var(--text-lg);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
  font-family: var(--font-mono);
}

.summary-value.fee {
  color: var(--warning);
}

.tx-section {
  margin-top: var(--space-6);
}

.section-title {
  font-size: var(--text-lg);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
  margin-bottom: var(--space-4);
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

.section-total {
  font-size: var(--text-sm);
  color: var(--text-tertiary);
  font-weight: var(--weight-normal);
  margin-left: auto;
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
  border-left-color: #ef4444;
}

.output-item {
  background: rgba(16, 185, 129, 0.08);
  border-left-color: #10b981;
}

.coinbase-badge {
  color: #fbbf24;
  font-weight: var(--weight-semibold);
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.coinbase-data {
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  color: var(--text-tertiary);
  padding: var(--space-2);
  background: var(--bg-elevated);
  border-radius: var(--radius-sm);
}

.regular-io {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.io-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--space-1);
}

.io-index {
  font-size: var(--text-sm);
  color: var(--text-tertiary);
  font-weight: var(--weight-medium);
}

.io-amount {
  font-family: var(--font-mono);
  font-size: var(--text-base);
  font-weight: var(--weight-bold);
  color: var(--text-accent);
}

.coldstake-box {
  padding: var(--space-3);
  background: rgba(102, 45, 145, 0.2);
  border: 1px solid rgba(102, 45, 145, 0.4);
  border-radius: var(--radius-sm);
  margin-top: var(--space-2);
}

.coldstake-title {
  color: #a78bfa;
  font-weight: var(--weight-bold);
  margin-bottom: var(--space-2);
  font-size: var(--text-sm);
}

.coldstake-address {
  font-size: var(--text-sm);
  margin-bottom: var(--space-1);
  display: flex;
  gap: var(--space-2);
  align-items: center;
}

.coldstake-address:last-child {
  margin-bottom: 0;
}

.coldstake-address code {
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  color: var(--text-primary);
  background: rgba(0, 0, 0, 0.3);
  padding: var(--space-1) var(--space-2);
  border-radius: var(--radius-xs);
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
}

.io-address,
.io-meta {
  font-size: var(--text-sm);
  display: flex;
  gap: var(--space-2);
  align-items: center;
}

.address-label,
.meta-label {
  color: var(--text-tertiary);
  font-size: var(--text-xs);
  white-space: nowrap;
}

.io-address code {
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
}

.meta-value {
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  color: var(--text-tertiary);
}

.spent-badge {
  padding: var(--space-1) var(--space-2);
  background: rgba(16, 185, 129, 0.2);
  color: var(--success);
  font-size: var(--text-xs);
  font-weight: var(--weight-bold);
  border-radius: var(--radius-xs);
  margin-left: auto;
}

.reward-banner {
  margin-top: var(--space-4);
  padding: var(--space-3);
  background: linear-gradient(135deg, rgba(89, 252, 179, 0.15), rgba(16, 185, 129, 0.15));
  border: 1px solid var(--success);
  border-radius: var(--radius-md);
  text-align: center;
  color: var(--text-primary);
  font-size: var(--text-base);
}

.reward-banner strong {
  color: var(--success);
  font-weight: var(--weight-extrabold);
  font-size: var(--text-lg);
}

@media (max-width: 768px) {
  .tx-card-header {
    flex-direction: column;
  }
  
  .tx-summary {
    width: 100%;
    justify-content: space-between;
  }
  
  .section-title {
    flex-direction: column;
    align-items: flex-start;
  }
  
  .section-total {
    margin-left: 0;
  }
}
</style>
