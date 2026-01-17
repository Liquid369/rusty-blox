<template>
  <div class="io-table">
    <div class="io-section">
      <h3 class="io-section-title">
        Inputs ({{ inputs.length }})
        <Badge variant="info" size="sm">{{ formatPIV(totalInput) }} PIV</Badge>
      </h3>
      <div class="io-list">
        <div v-for="(input, index) in inputs" :key="index" class="io-item">
          <div class="io-index">{{ index }}</div>
          <div class="io-details">
            <div v-if="input.coinbase" class="io-coinbase">
              <Badge variant="info" size="sm">Coinbase</Badge>
              <span class="font-mono">{{ input.coinbase }}</span>
            </div>
            <div v-else class="io-content">
              <div v-if="input.addresses && input.addresses.length > 0" class="io-address">
                <HashDisplay 
                  :hash="input.addresses[0]"
                  :link-to="`/address/${input.addresses[0]}`"
                  :start-length="12"
                  :end-length="12"
                />
              </div>
              <div v-if="input.txid" class="io-txid">
                <span class="io-label">From TX:</span>
                <HashDisplay 
                  :hash="input.txid"
                  :link-to="`/tx/${input.txid}`"
                  :start-length="8"
                  :end-length="8"
                />
                <span class="io-vout">[{{ input.vout }}]</span>
              </div>
            </div>
            <div v-if="input.value" class="io-value">
              {{ formatPIV(input.value) }} PIV
            </div>
          </div>
        </div>
      </div>
    </div>

    <div class="io-arrow">→</div>

    <div class="io-section">
      <h3 class="io-section-title">
        Outputs ({{ outputs.length }})
        <Badge variant="success" size="sm">{{ formatPIV(totalOutput) }} PIV</Badge>
      </h3>
      <div class="io-list">
        <div v-for="(output, index) in outputs" :key="index" class="io-item">
          <div class="io-index">{{ index }}</div>
          <div class="io-details">
            <div class="io-content">
              <div v-if="output.addresses && output.addresses.length > 0" class="io-address">
                <HashDisplay 
                  :hash="output.addresses[0]"
                  :link-to="`/address/${output.addresses[0]}`"
                  :start-length="12"
                  :end-length="12"
                />
              </div>
              <div v-else class="io-unspendable">
                <Badge variant="default" size="sm">{{ output.type || 'Unspendable' }}</Badge>
              </div>
              <div v-if="output.type === 'coldstake'" class="io-coldstake">
                <Badge variant="accent" size="sm">Cold Stake</Badge>
              </div>
            </div>
            <div class="io-value">
              {{ formatPIV(output.value) }} PIV
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>

  <div v-if="fees" class="io-fees">
    <InfoRow label="Transaction Fee">
      <span class="fee-amount">{{ formatPIV(fees) }} PIV</span>
    </InfoRow>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import { formatPIV } from '@/utils/formatters'
import Badge from './Badge.vue'
import HashDisplay from './HashDisplay.vue'
import InfoRow from './InfoRow.vue'

const props = defineProps({
  inputs: {
    type: Array,
    required: true
  },
  outputs: {
    type: Array,
    required: true
  },
  fees: {
    type: String,
    default: ''
  }
})

const totalInput = computed(() => {
  return props.inputs.reduce((sum, input) => {
    const value = parseFloat(input.value || 0)
    return sum + value
  }, 0).toString()
})

const totalOutput = computed(() => {
  return props.outputs.reduce((sum, output) => {
    const value = parseFloat(output.value || 0)
    return sum + value
  }, 0).toString()
})
</script>

<style scoped>
.io-table {
  display: grid;
  grid-template-columns: 1fr auto 1fr;
  gap: var(--space-6);
  margin: var(--space-6) 0;
}

.io-section {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.io-section-title {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  font-size: var(--text-lg);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
  padding-bottom: var(--space-3);
  border-bottom: 2px solid var(--border-primary);
}

.io-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.io-item {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: var(--space-3);
  padding: var(--space-3);
  background: var(--bg-tertiary);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
}

.io-index {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  background: var(--bg-secondary);
  border-radius: var(--radius-sm);
  font-family: var(--font-mono);
  font-size: var(--text-sm);
  font-weight: var(--weight-bold);
  color: var(--text-secondary);
}

.io-details {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.io-content {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.io-coinbase {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--text-sm);
}

.io-address {
  word-break: break-all;
}

.io-txid {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--text-xs);
  color: var(--text-tertiary);
}

.io-label {
  font-weight: var(--weight-medium);
}

.io-vout {
  font-family: var(--font-mono);
  color: var(--text-secondary);
}

.io-value {
  font-family: var(--font-mono);
  font-size: var(--text-lg);
  font-weight: var(--weight-bold);
  color: var(--pivx-accent);
}

.io-unspendable {
  color: var(--text-tertiary);
  font-size: var(--text-sm);
}

.io-coldstake {
  margin-top: var(--space-1);
}

.io-arrow {
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: var(--text-4xl);
  color: var(--text-accent);
  padding-top: 60px;
}

.io-fees {
  margin-top: var(--space-6);
  padding: var(--space-4);
  background: var(--bg-secondary);
  border: 2px solid var(--border-secondary);
  border-radius: var(--radius-lg);
}

.fee-amount {
  font-family: var(--font-mono);
  font-size: var(--text-lg);
  font-weight: var(--weight-bold);
  color: var(--warning);
}

@media (max-width: 1024px) {
  .io-table {
    grid-template-columns: 1fr;
    gap: var(--space-4);
  }

  .io-arrow {
    transform: rotate(90deg);
    padding: 0;
  }
}

@media (max-width: 768px) {
  .io-item {
    grid-template-columns: 1fr;
  }

  .io-index {
    width: 100%;
    height: auto;
    padding: var(--space-2);
  }
}
</style>
