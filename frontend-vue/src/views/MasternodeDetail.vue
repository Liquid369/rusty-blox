<template>
  <AppLayout>
    <div class="masternode-detail-page">
      <div v-if="loading" class="skeleton" style="height: 400px;"></div>

      <div v-else-if="error" class="state-container">
        <h2>Failed to load masternode</h2>
        <p class="error-text">{{ error }}</p>
        <UiButton @click="loadMasternode">Try Again</UiButton>
      </div>

      <div v-else-if="mn">
        <h1>Masternode #{{ mn.rank }}</h1>
        <span :class="['badge', statusBadgeClass(mn.status)]">{{ mn.status }}</span>

        <UiCard class="mt-6">
          <template #header>
            <h2>Overview</h2>
          </template>

          <div class="detail-grid">
            <div class="detail-row">
              <span class="detail-label">Status</span>
              <span class="detail-value">
                <span :class="['badge', statusBadgeClass(mn.status)]">{{ mn.status }}</span>
              </span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Rank</span>
              <span class="detail-value">{{ mn.rank }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Address</span>
              <router-link v-if="mn.addr" :to="`/address/${mn.addr}`" class="detail-value mono link">
                {{ mn.addr }}
              </router-link>
              <span v-else class="detail-value text-tertiary">—</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Collateral</span>
              <router-link :to="`/tx/${mn.txhash}`" class="detail-value mono link">
                {{ mn.txhash }}:{{ mn.outidx }}
              </router-link>
            </div>
            <div class="detail-row">
              <span class="detail-label">Type</span>
              <span class="detail-value">{{ mn.type || '—' }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Network</span>
              <span class="detail-value network">{{ mn.network || '—' }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Protocol Version</span>
              <span class="detail-value mono">{{ mn.version }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Active Since</span>
              <span class="detail-value">{{ formatActiveSince(mn.activetime) }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Active Duration</span>
              <span class="detail-value">{{ formatDuration(mn.activetime) }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Last Seen</span>
              <span class="detail-value">{{ formatTimestamp(mn.lastseen) }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Last Paid</span>
              <span class="detail-value">{{ formatTimestamp(getLastPaidSeconds(mn)) }}</span>
            </div>
          </div>
        </UiCard>
      </div>

      <div v-else class="state-container">
        <h2>Masternode not found</h2>
        <p>The requested masternode was not found in the current list.</p>
        <UiButton @click="$router.push('/masternodes')">View All Masternodes</UiButton>
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import { masternodeService } from '@/services/masternodeService'
import AppLayout from '@/components/layout/AppLayout.vue'
import UiCard from '@/components/common/UiCard.vue'
import UiButton from '@/components/common/UiButton.vue'

const route = useRoute()

const loading = ref(true)
const error = ref('')
const mn = ref(null)

const statusBadgeClass = (status) => {
  const classes = {
    ENABLED: 'badge-success',
    PRE_ENABLED: 'badge-info',
    EXPIRED: 'badge-warning',
    REMOVE: 'badge-danger'
  }
  return classes[status] || 'badge-info'
}

const getLastPaidSeconds = (row) => {
  const lastPaid = Number(row?.lastpaid)
  if (Number.isFinite(lastPaid) && lastPaid > 0) return lastPaid
  const lastSeen = Number(row?.lastseen)
  if (Number.isFinite(lastSeen) && lastSeen > 0) return lastSeen
  return null
}

const formatTimestamp = (seconds) => {
  if (!seconds) return '—'
  return new Date(seconds * 1000).toLocaleString()
}

const formatActiveSince = (activeSeconds) => {
  const total = Number(activeSeconds)
  if (!Number.isFinite(total) || total <= 0) return '—'
  const since = Math.floor(Date.now() / 1000) - total
  return new Date(since * 1000).toLocaleString()
}

const formatDuration = (seconds) => {
  const total = Number(seconds)
  if (!Number.isFinite(total) || total <= 0) return '—'

  const days = Math.floor(total / 86400)
  const hours = Math.floor((total % 86400) / 3600)
  const minutes = Math.floor((total % 3600) / 60)

  if (days > 0) return `${days}d ${hours}h`
  if (hours > 0) return `${hours}h ${minutes}m`
  return `${minutes}m`
}

const loadMasternode = async () => {
  loading.value = true
  error.value = ''
  mn.value = null

  const id = String(route.params.id || '')

  try {
    const list = await masternodeService.getMasternodeList()
    const rows = Array.isArray(list) ? list : []

    // Route param is "txhash-outidx"; fall back to address/txhash lookup
    mn.value =
      rows.find((row) => `${row.txhash}-${row.outidx}` === id) ||
      rows.find((row) => row.txhash === id || row.addr === id || row.pubkey === id) ||
      null
  } catch (err) {
    error.value = 'Failed to load the masternode list.'
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  loadMasternode()
})
</script>

<style scoped>
.masternode-detail-page {
  animation: fadeIn 0.3s ease;
}

.masternode-detail-page h1 {
  font-size: var(--text-4xl);
  font-weight: var(--weight-extrabold);
  margin-bottom: var(--space-4);
  color: var(--text-primary);
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

.detail-value.link {
  color: var(--text-accent);
  text-decoration: none;
}

.detail-value.link:hover {
  color: var(--pivx-accent-dark);
  text-decoration: underline;
}

.network {
  text-transform: uppercase;
}

.mono {
  font-family: var(--font-mono);
}

.state-container {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-4);
  text-align: center;
  padding: var(--space-16) var(--space-6);
}

.error-text {
  color: var(--danger);
}

@keyframes fadeIn {
  from {
    opacity: 0;
    transform: translateY(10px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
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
