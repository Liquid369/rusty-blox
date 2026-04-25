<template>
  <AppLayout>
    <div class="masternode-list-page">
      <div class="page-header">
        <h1>Masternodes</h1>
        <p class="page-subtitle">Active PIVX masternodes securing the network</p>
      </div>

      <!-- Stats Cards -->
      <div class="stats-grid">
        <Card v-if="mnCount">
          <template #header>Total</template>
          <div class="stat-value">{{ formatNumber(mnCount.total) }}</div>
        </Card>
        <Card v-if="mnCount">
          <template #header>Enabled</template>
          <div class="stat-value">{{ formatNumber(mnCount.enabled) }}</div>
        </Card>
        <Card v-if="mnCount">
          <template #header>IPv4</template>
          <div class="stat-value">{{ formatNumber(mnCount.ipv4) }}</div>
        </Card>
        <Card v-if="mnCount">
          <template #header>IPv6</template>
          <div class="stat-value">{{ formatNumber(mnCount.ipv6) }}</div>
        </Card>
        <Card v-if="mnCount">
          <template #header>Onion</template>
          <div class="stat-value">{{ formatNumber(mnCount.onion) }}</div>
        </Card>
      </div>

      <!-- Loading State -->
      <div v-if="loading && masternodes.length === 0" class="loading-container">
        <LoadingSpinner size="lg" text="Loading masternodes..." />
      </div>

      <!-- Error State -->
      <div v-else-if="error" class="error-container">
        <EmptyState
          icon="⚠️"
          title="Failed to Load Masternodes"
          :message="error"
        >
          <template #action>
            <Button @click="fetchMasternodes">Try Again</Button>
          </template>
        </EmptyState>
      </div>

      <!-- Masternodes Table -->
      <div v-else>
        <!-- Filters -->
        <Card class="filters-card">
          <div class="filters">
            <div class="filter-group">
              <label>Status</label>
              <select v-model="statusFilter" class="filter-select">
                <option value="all">All</option>
                <option value="ENABLED">Enabled</option>
                <option value="PRE_ENABLED">Pre-Enabled</option>
                <option value="EXPIRED">Expired</option>
                <option value="REMOVE">Remove</option>
              </select>
            </div>
            <div class="filter-group">
              <label>Protocol</label>
              <select v-model="protocolFilter" class="filter-select">
                <option value="all">All Protocols</option>
                <option value="70926">70926</option>
              </select>
            </div>
            <div class="filter-group">
              <label>Search</label>
              <input
                v-model="searchQuery"
                type="text"
                placeholder="Address, IP, or TxHash..."
                class="filter-input"
              />
            </div>
          </div>
        </Card>

        <!-- Table -->
        <Card class="table-card">
          <div class="table-container">
            <table class="mn-table">
              <thead>
                <tr>
                  <th @click="sortBy('rank')" class="sortable">
                    Rank <span class="sort-icon">{{ getSortIcon('rank') }}</span>
                  </th>
                  <th>Status</th>
                  <th>Address</th>
                  <th @click="sortBy('activetime')" class="sortable">
                    Active Since <span class="sort-icon">{{ getSortIcon('activetime') }}</span>
                  </th>
                  <th>Duration</th>
                  <th @click="sortBy('lastpaid')" class="sortable">
                    Last Paid <span class="sort-icon">{{ getSortIcon('lastpaid') }}</span>
                  </th>
                  <th>Protocol</th>
                </tr>
              </thead>
              <tbody>
                <tr
                  v-for="mn in paginatedMasternodes"
                  :key="mn.txhash + '-' + mn.outidx"
                  class="mn-row"
                >
                  <td class="rank">
                    <a class="mn-link" @click.prevent="goToMasternode(mn)">{{ mn.rank }}</a>
                  </td>
                  <td>
                    <Badge :variant="getStatusVariant(mn.status)">
                      {{ mn.status }}
                    </Badge>
                  </td>
                  <td class="address">
                    <HashDisplay
                      v-if="looksLikePivxAddress(mn.addr)"
                      :hash="mn.addr"
                      :truncate="true"
                      :start-length="8"
                      :end-length="6"
                      show-copy
                      :link-to="`/address/${mn.addr}`"
                    />
                    <span v-else>{{ mn.addr || '—' }}</span>
                  </td>
                  <td>{{ formatActiveSince(mn.activetime) }}</td>
                  <td>{{ formatDuration(mn.activetime) }}</td>
                  <td>{{ formatLastPaid(mn) }}</td>
                  <td class="protocol">{{ mn.version }}</td>
                </tr>
              </tbody>
            </table>
          </div>

          <!-- Empty State -->
          <EmptyState
            v-if="filteredMasternodes.length === 0"
            icon="🔍"
            title="No Masternodes Found"
            message="Try adjusting your filters"
          />

          <!-- Pagination -->
          <Pagination
            v-if="filteredMasternodes.length > 0"
            :current-page="currentPage"
            :page-size="pageSize"
            :total="filteredMasternodes.length"
            @update:page="currentPage = $event"
            @update:page-size="pageSize = $event; currentPage = 1"
          />
        </Card>
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { masternodeService } from '@/services/masternodeService'
import { formatNumber, formatTimeAgo, formatDuration } from '@/utils/formatters'
import AppLayout from '@/components/layout/AppLayout.vue'
import Card from '@/components/common/Card.vue'
import Badge from '@/components/common/Badge.vue'
import Button from '@/components/common/Button.vue'
import HashDisplay from '@/components/common/HashDisplay.vue'
import LoadingSpinner from '@/components/common/LoadingSpinner.vue'
import EmptyState from '@/components/common/EmptyState.vue'
import Pagination from '@/components/common/Pagination.vue'

const router = useRouter()

const mnCount = ref(null)
const masternodes = ref([])
const loading = ref(false)
const error = ref('')

// Filters
const statusFilter = ref('all')
const protocolFilter = ref('all')
const searchQuery = ref('')

// Sorting
const sortField = ref('rank')
const sortDirection = ref('asc')

// Pagination
const currentPage = ref(1)
const pageSize = ref(50)

const getStatusVariant = (status) => {
  const variants = {
    'ENABLED': 'success',
    'PRE_ENABLED': 'info',
    'EXPIRED': 'warning',
    'REMOVE': 'danger'
  }
  return variants[status] || 'secondary'
}

const looksLikePivxAddress = (value) => {
  if (typeof value !== 'string') return false
  // PIVX transparent addresses are typically base58 and start with 'D'.
  return value.startsWith('D') && value.length >= 30 && value.length <= 40
}

const getPayeeAddress = (mn) => {
  if (looksLikePivxAddress(mn?.payee)) return mn.payee
  if (looksLikePivxAddress(mn?.pubkey)) return mn.pubkey
  // Some backends may put the payee address in 'addr'. Only use it if it looks like an address.
  if (looksLikePivxAddress(mn?.addr) && !String(mn.addr).includes('.') && !String(mn.addr).includes(':')) {
    return mn.addr
  }
  return null
}

const filteredMasternodes = computed(() => {
  let filtered = [...masternodes.value]

  // Status filter
  if (statusFilter.value !== 'all') {
    filtered = filtered.filter(mn => mn.status === statusFilter.value)
  }

  // Protocol filter
  if (protocolFilter.value !== 'all') {
    filtered = filtered.filter(mn => mn.version.toString() === protocolFilter.value)
  }

  // Search filter
  if (searchQuery.value) {
    const query = searchQuery.value.toLowerCase()
    filtered = filtered.filter(mn =>
      (getPayeeAddress(mn) || '').toLowerCase().includes(query) ||
      (mn.pubkey || '').toLowerCase().includes(query) ||
      (mn.addr || '').toLowerCase().includes(query) ||
      (mn.txhash || '').toLowerCase().includes(query)
    )
  }

  // Sort
  filtered.sort((a, b) => {
    let aVal = a[sortField.value]
    let bVal = b[sortField.value]

    if (sortField.value === 'lastpaid') {
      aVal = getLastPaidSeconds(a) ?? 0
      bVal = getLastPaidSeconds(b) ?? 0
    }

    if (sortField.value === 'rank') {
      aVal = parseInt(aVal) || 0
      bVal = parseInt(bVal) || 0
    }

    if (sortField.value === 'activetime') {
      aVal = parseInt(aVal) || 0
      bVal = parseInt(bVal) || 0
    }

    if (sortField.value === 'lastseen') {
      aVal = parseInt(aVal) || 0
      bVal = parseInt(bVal) || 0
    }

    if (sortDirection.value === 'asc') {
      return aVal > bVal ? 1 : -1
    } else {
      return aVal < bVal ? 1 : -1
    }
  })

  return filtered
})

const paginatedMasternodes = computed(() => {
  const start = (currentPage.value - 1) * pageSize.value
  const end = start + pageSize.value
  return filteredMasternodes.value.slice(start, end)
})

const sortBy = (field) => {
  if (sortField.value === field) {
    sortDirection.value = sortDirection.value === 'asc' ? 'desc' : 'asc'
  } else {
    sortField.value = field
    sortDirection.value = 'asc'
  }
}

const getLastPaidSeconds = (mn) => {
  // Backend returns unix seconds (UTC).
  // Prefer lastpaid, then lastpaidtime, then fall back to lastseen.
  const lastPaid = Number(mn?.lastpaid)
  if (Number.isFinite(lastPaid) && lastPaid > 0) return lastPaid

  const lastPaidTime = Number(mn?.lastpaidtime)
  if (Number.isFinite(lastPaidTime) && lastPaidTime > 0) return lastPaidTime

  const lastSeen = Number(mn?.lastseen)
  if (Number.isFinite(lastSeen) && lastSeen > 0) return lastSeen

  return null
}

const formatLastPaid = (mn) => {
  const seconds = getLastPaidSeconds(mn)
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

const goToMasternode = (mn) => {
  const id = `${mn.txhash}-${mn.outidx}`
  router.push(`/masternode/${id}`)
}

const getSortIcon = (field) => {
  if (sortField.value !== field) return '↕'
  return sortDirection.value === 'asc' ? '↑' : '↓'
}

const fetchMasternodes = async () => {
  loading.value = true
  error.value = ''

  try {
    // Fetch count and list in parallel
    const [countData, listData] = await Promise.all([
      masternodeService.getMasternodeCount(),
      masternodeService.getMasternodeList()
    ])

    mnCount.value = countData
    masternodes.value = listData
  } catch (err) {
    console.error('Failed to fetch masternodes:', err)
    error.value = err.message || 'Failed to load masternodes'
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  fetchMasternodes()
})
</script>

<style scoped>
.masternode-list-page {
  padding: var(--space-6);
  max-width: 1600px;
  margin: 0 auto;
}

.page-header {
  margin-bottom: var(--space-6);
}

.page-subtitle {
  color: var(--text-secondary);
  font-size: var(--text-lg);
  margin-top: var(--space-2);
}

.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: var(--space-4);
  margin-bottom: var(--space-6);
}

.stat-value {
  font-size: var(--text-3xl);
  font-weight: 700;
  color: var(--text-accent);
  margin-top: var(--space-2);
}

.filters-card {
  margin-bottom: var(--space-4);
}

.filters {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
  gap: var(--space-4);
}

.filter-group {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.filter-group label {
  font-size: var(--text-sm);
  color: var(--text-secondary);
  text-transform: uppercase;
  font-weight: 600;
}

.filter-select,
.filter-input {
  padding: var(--space-3);
  background: var(--bg-tertiary);
  border: 2px solid var(--border-secondary);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-family: var(--font-primary);
  font-size: var(--text-base);
}

.filter-select:focus,
.filter-input:focus {
  outline: none;
  border-color: var(--border-accent);
}

.table-card {
  overflow: hidden;
}

.table-container {
  overflow-x: auto;
}

.mn-table {
  width: 100%;
  border-collapse: collapse;
}

.mn-table thead th {
  background: var(--bg-tertiary);
  color: var(--text-secondary);
  font-size: var(--text-xs);
  font-weight: 600;
  text-transform: uppercase;
  padding: var(--space-3) var(--space-4);
  text-align: left;
  border-bottom: 2px solid var(--border-secondary);
}

.mn-table thead th.sortable {
  cursor: pointer;
  user-select: none;
}

.mn-table thead th.sortable:hover {
  color: var(--text-accent);
}

.sort-icon {
  margin-left: var(--space-1);
  color: var(--text-tertiary);
}

.mn-table tbody tr {
  border-bottom: 1px solid var(--border-subtle);
  transition: background 0.2s;
}

.mn-table tbody tr:hover {
  background: var(--bg-tertiary);
}

.mn-table tbody td {
  padding: var(--space-3) var(--space-4);
  font-size: var(--text-sm);
}

.mn-table .rank {
  font-weight: 600;
  color: var(--text-accent);
}

.mn-link {
  color: inherit;
  text-decoration: none;
  cursor: pointer;
}

.mn-link:hover {
  text-decoration: underline;
}

.mn-table .ip-address {
  font-family: var(--font-mono);
  font-size: var(--text-xs);
}

.mn-table .protocol {
  font-family: var(--font-mono);
  color: var(--text-secondary);
}

.loading-container,
.error-container {
  min-height: 400px;
  display: flex;
  align-items: center;
  justify-content: center;
}

@media (max-width: 768px) {
  .masternode-list-page {
    padding: var(--space-4);
  }

  .stats-grid {
    grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
  }

  .filters {
    grid-template-columns: 1fr;
  }

  .table-container {
    font-size: var(--text-xs);
  }

  .mn-table thead th,
  .mn-table tbody td {
    padding: var(--space-2) var(--space-3);
  }
}
</style>
