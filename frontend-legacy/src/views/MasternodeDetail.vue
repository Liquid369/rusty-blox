<template>
  <AppLayout>
    <div class="page">
      <div class="page-header">
        <h1>Masternode</h1>
        <p class="page-subtitle" v-if="mn">Collateral: {{ mn.txhash }}:{{ mn.outidx }}</p>
        <p class="page-subtitle" v-else>Collateral: {{ id }}</p>
      </div>

      <div v-if="loading" class="loading">
        <LoadingSpinner size="lg" text="Loading masternode..." />
      </div>

      <div v-else-if="error" class="error">
        <EmptyState icon="⚠️" title="Failed to Load Masternode" :message="error">
          <template #action>
            <Button @click="fetchMasternode">Try Again</Button>
          </template>
        </EmptyState>
      </div>

      <div v-else-if="!mn" class="error">
        <EmptyState icon="🔍" title="Masternode Not Found" message="The requested masternode was not found in the current list." />
      </div>

      <div v-else class="content">
        <Card>
          <template #header>Details</template>
          <div class="details-grid">
            <div class="label">Status</div>
            <div>
              <Badge :variant="getStatusVariant(mn.status)">{{ mn.status }}</Badge>
            </div>

            <div class="label">Address</div>
            <div>
              <HashDisplay
                v-if="looksLikePivxAddress(mn.addr)"
                :hash="mn.addr"
                :truncate="true"
                :start-length="10"
                :end-length="10"
                show-copy
                :link-to="`/address/${mn.addr}`"
              />
              <span v-else class="mono">{{ mn.addr || '—' }}</span>
            </div>

            <div class="label">Active Since</div>
            <div class="mono">{{ formatActiveSince(mn.activetime) }}</div>

            <div class="label">Protocol</div>
            <div class="mono">{{ mn.version }}</div>

            <div class="label">Rank</div>
            <div class="mono">{{ mn.rank }}</div>

            <div class="label">Duration</div>
            <div class="mono">{{ formatDuration(mn.activetime) }}</div>

            <div class="label">Last Paid</div>
            <div class="mono">{{ formatLastPaid(mn) }}</div>
          </div>
        </Card>
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue'
import { useRoute } from 'vue-router'
import AppLayout from '@/components/layout/AppLayout.vue'
import Card from '@/components/common/Card.vue'
import Badge from '@/components/common/Badge.vue'
import Button from '@/components/common/Button.vue'
import EmptyState from '@/components/common/EmptyState.vue'
import HashDisplay from '@/components/common/HashDisplay.vue'
import LoadingSpinner from '@/components/common/LoadingSpinner.vue'
import { masternodeService } from '@/services/masternodeService'
import { formatDuration, formatTimeAgo } from '@/utils/formatters'

const route = useRoute()

const id = computed(() => String(route.params.id || ''))

const mn = ref(null)
const loading = ref(false)
const error = ref('')

const looksLikePivxAddress = (value) => {
  if (typeof value !== 'string') return false
  return value.startsWith('D') && value.length >= 30 && value.length <= 40
}

const payeeAddress = computed(() => {
  const row = mn.value
  if (!row) return null
  if (looksLikePivxAddress(row.payee)) return row.payee
  if (looksLikePivxAddress(row.pubkey)) return row.pubkey
  if (looksLikePivxAddress(row.addr) && !String(row.addr).includes('.') && !String(row.addr).includes(':')) return row.addr
  return null
})

const getStatusVariant = (status) => {
  const variants = {
    'ENABLED': 'success',
    'PRE_ENABLED': 'info',
    'EXPIRED': 'warning',
    'REMOVE': 'danger'
  }
  return variants[status] || 'secondary'
}

const getLastPaidSeconds = (mnRow) => {
  const lastPaid = Number(mnRow?.lastpaid)
  if (Number.isFinite(lastPaid) && lastPaid > 0) return lastPaid

  const lastPaidTime = Number(mnRow?.lastpaidtime)
  if (Number.isFinite(lastPaidTime) && lastPaidTime > 0) return lastPaidTime

  const lastSeen = Number(mnRow?.lastseen)
  if (Number.isFinite(lastSeen) && lastSeen > 0) return lastSeen

  return null
}

const formatLastPaid = (mnRow) => {
  const seconds = getLastPaidSeconds(mnRow)
  if (!seconds) return '—'

  const utc = new Date(seconds * 1000).toISOString().replace('T', ' ').replace('.000Z', ' UTC')
  return `${utc} (${formatTimeAgo(seconds)})`
}

const formatActiveSince = (activeSeconds) => {
  if (!activeSeconds || activeSeconds <= 0) return '—'
  
  // Calculate the timestamp when the masternode became active
  const now = Math.floor(Date.now() / 1000)
  const activeSinceSeconds = now - activeSeconds
  
  // Format as UTC datetime
  return new Date(activeSinceSeconds * 1000).toISOString().replace('T', ' ').replace('.000Z', ' UTC')
}

const fetchMasternode = async () => {
  loading.value = true
  error.value = ''
  mn.value = null

  try {
    const list = await masternodeService.getMasternodeList()
    mn.value = (list || []).find((row) => `${row.txhash}-${row.outidx}` === id.value) || null
  } catch (err) {
    console.error('Failed to fetch masternode:', err)
    error.value = err.message || 'Failed to load masternode'
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  fetchMasternode()
})
</script>

<style scoped>
.page {
  padding: var(--space-6);
  max-width: 1100px;
  margin: 0 auto;
}

.page-header {
  margin-bottom: var(--space-6);
}

.page-subtitle {
  color: var(--text-secondary);
  font-size: var(--text-sm);
  margin-top: var(--space-2);
  font-family: var(--font-mono);
}

.loading,
.error {
  min-height: 300px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.details-grid {
  display: grid;
  grid-template-columns: 180px 1fr;
  gap: var(--space-3) var(--space-4);
  align-items: start;
}

.label {
  color: var(--text-secondary);
  font-size: var(--text-xs);
  font-weight: 600;
  text-transform: uppercase;
}

.mono {
  font-family: var(--font-mono);
}

@media (max-width: 768px) {
  .details-grid {
    grid-template-columns: 1fr;
  }
}
</style>
