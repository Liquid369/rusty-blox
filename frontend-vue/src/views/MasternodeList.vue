<template>
  <AppLayout>
    <div class="masternode-list-page">
      <h1>Masternodes</h1>
      <p class="page-subtitle">Active PIVX masternodes securing the network</p>

      <!-- Stats -->
      <div class="stats-grid">
        <StatCard label="Total" :value="mnCount?.total" format="number" :loading="loading" />
        <StatCard label="Enabled" :value="mnCount?.enabled" format="number" :loading="loading" />
        <StatCard label="IPv4" :value="mnCount?.ipv4" format="number" :loading="loading" />
        <StatCard label="IPv6" :value="mnCount?.ipv6" format="number" :loading="loading" />
        <StatCard label="Onion" :value="mnCount?.onion" format="number" :loading="loading" />
      </div>

      <!-- Loading -->
      <div v-if="loading" class="state-container">
        <span class="loading-spinner"></span>
        <p>Loading masternodes...</p>
      </div>

      <!-- Error -->
      <div v-else-if="error" class="state-container error-state">
        <h2>Failed to load masternodes</h2>
        <p>{{ error }}</p>
        <UiButton @click="fetchMasternodes">Try Again</UiButton>
      </div>

      <template v-else>
        <!-- Filters -->
        <UiCard class="filters-card">
          <div class="filters">
            <div class="filter-group">
              <label for="mn-status">Status</label>
              <select id="mn-status" v-model="statusFilter" class="filter-select">
                <option value="all">All</option>
                <option v-for="status in statusOptions" :key="status" :value="status">
                  {{ status }}
                </option>
              </select>
            </div>
            <div class="filter-group">
              <label for="mn-network">Network</label>
              <select id="mn-network" v-model="networkFilter" class="filter-select">
                <option value="all">All Networks</option>
                <option value="ipv4">IPv4</option>
                <option value="ipv6">IPv6</option>
                <option value="onion">Onion</option>
              </select>
            </div>
            <div class="filter-group">
              <label for="mn-search">Search</label>
              <input
                id="mn-search"
                v-model="searchQuery"
                type="text"
                placeholder="Address or collateral txhash..."
                class="filter-input"
              />
            </div>
          </div>
        </UiCard>

        <!-- Table -->
        <UiCard class="table-card">
          <div v-if="filteredMasternodes.length === 0" class="empty-note">
            <p>No masternodes match your filters.</p>
          </div>

          <template v-else>
            <div class="table-container">
              <table class="mn-table">
                <thead>
                  <tr>
                    <th class="sortable" @click="sortBy('rank')">
                      Rank <span class="sort-icon">{{ getSortIcon('rank') }}</span>
                    </th>
                    <th>Status</th>
                    <th>Address</th>
                    <th>Network</th>
                    <th class="sortable" @click="sortBy('activetime')">
                      Active <span class="sort-icon">{{ getSortIcon('activetime') }}</span>
                    </th>
                    <th class="sortable" @click="sortBy('lastseen')">
                      Last Seen <span class="sort-icon">{{ getSortIcon('lastseen') }}</span>
                    </th>
                    <th class="sortable" @click="sortBy('lastpaid')">
                      Last Paid <span class="sort-icon">{{ getSortIcon('lastpaid') }}</span>
                    </th>
                    <th>Protocol</th>
                  </tr>
                </thead>
                <tbody>
                  <tr
                    v-for="mn in paginatedMasternodes"
                    :key="`${mn.txhash}-${mn.outidx}`"
                    class="mn-row"
                    @click="goToMasternode(mn)"
                  >
                    <td class="rank">{{ mn.rank }}</td>
                    <td>
                      <span :class="['badge', statusBadgeClass(mn.status)]">{{ mn.status }}</span>
                    </td>
                    <td class="address">
                      <router-link
                        v-if="mn.addr"
                        :to="`/address/${mn.addr}`"
                        class="address-link mono"
                        @click.stop
                      >
                        {{ truncateMiddle(mn.addr) }}
                      </router-link>
                      <span v-else class="text-tertiary">—</span>
                    </td>
                    <td class="network">{{ mn.network || '—' }}</td>
                    <td>{{ formatDuration(mn.activetime) }}</td>
                    <td>{{ formatTimeAgo(mn.lastseen) }}</td>
                    <td>{{ formatTimeAgo(getLastPaidSeconds(mn)) }}</td>
                    <td class="protocol">{{ mn.version }}</td>
                  </tr>
                </tbody>
              </table>
            </div>

            <!-- Pagination -->
            <div class="pagination">
              <span class="pagination-info">
                Showing {{ pageStart + 1 }}–{{ pageEnd }} of {{ filteredMasternodes.length }}
              </span>
              <div class="pagination-controls">
                <UiButton variant="secondary" :disabled="currentPage === 1" @click="currentPage--">
                  ← Prev
                </UiButton>
                <span class="pagination-page">Page {{ currentPage }} / {{ totalPages }}</span>
                <UiButton
                  variant="secondary"
                  :disabled="currentPage >= totalPages"
                  @click="currentPage++"
                >
                  Next →
                </UiButton>
              </div>
            </div>
          </template>
        </UiCard>
      </template>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, computed, watch, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { masternodeService } from '@/services/masternodeService'
import AppLayout from '@/components/layout/AppLayout.vue'
import StatCard from '@/components/common/StatCard.vue'
import UiCard from '@/components/common/UiCard.vue'
import UiButton from '@/components/common/UiButton.vue'

const router = useRouter()

const mnCount = ref(null)
const masternodes = ref([])
const loading = ref(true)
const error = ref('')

// Filters
const statusFilter = ref('all')
const networkFilter = ref('all')
const searchQuery = ref('')

// Sorting
const sortField = ref('rank')
const sortDirection = ref('asc')

// Pagination
const currentPage = ref(1)
const pageSize = 50

const statusOptions = computed(() => {
  const statuses = new Set(masternodes.value.map((mn) => mn.status).filter(Boolean))
  return [...statuses].sort()
})

const statusBadgeClass = (status) => {
  const classes = {
    ENABLED: 'badge-success',
    PRE_ENABLED: 'badge-info',
    EXPIRED: 'badge-warning',
    REMOVE: 'badge-danger'
  }
  return classes[status] || 'badge-info'
}

const truncateMiddle = (value, start = 10, end = 6) => {
  if (!value) return ''
  if (value.length <= start + end + 3) return value
  return `${value.slice(0, start)}...${value.slice(-end)}`
}

const getLastPaidSeconds = (mn) => {
  const lastPaid = Number(mn?.lastpaid)
  if (Number.isFinite(lastPaid) && lastPaid > 0) return lastPaid
  const lastSeen = Number(mn?.lastseen)
  if (Number.isFinite(lastSeen) && lastSeen > 0) return lastSeen
  return null
}

const formatTimeAgo = (timestamp) => {
  if (!timestamp) return '—'
  const diff = Date.now() - timestamp * 1000
  const minutes = Math.floor(diff / 60000)

  if (minutes < 1) return 'just now'
  if (minutes < 60) return `${minutes} min${minutes > 1 ? 's' : ''} ago`

  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours} hour${hours > 1 ? 's' : ''} ago`

  const days = Math.floor(hours / 24)
  return `${days} day${days > 1 ? 's' : ''} ago`
}

const formatDuration = (seconds) => {
  const total = Number(seconds)
  if (!Number.isFinite(total) || total <= 0) return '—'

  const days = Math.floor(total / 86400)
  const hours = Math.floor((total % 86400) / 3600)

  if (days > 0) return `${days}d ${hours}h`
  const minutes = Math.floor((total % 3600) / 60)
  if (hours > 0) return `${hours}h ${minutes}m`
  return `${minutes}m`
}

const filteredMasternodes = computed(() => {
  let filtered = [...masternodes.value]

  if (statusFilter.value !== 'all') {
    filtered = filtered.filter((mn) => mn.status === statusFilter.value)
  }

  if (networkFilter.value !== 'all') {
    filtered = filtered.filter((mn) => mn.network === networkFilter.value)
  }

  if (searchQuery.value) {
    const query = searchQuery.value.toLowerCase()
    filtered = filtered.filter(
      (mn) =>
        (mn.addr || '').toLowerCase().includes(query) ||
        (mn.pubkey || '').toLowerCase().includes(query) ||
        (mn.txhash || '').toLowerCase().includes(query)
    )
  }

  filtered.sort((a, b) => {
    let aVal
    let bVal

    if (sortField.value === 'lastpaid') {
      aVal = getLastPaidSeconds(a) ?? 0
      bVal = getLastPaidSeconds(b) ?? 0
    } else {
      aVal = Number(a[sortField.value]) || 0
      bVal = Number(b[sortField.value]) || 0
    }

    return sortDirection.value === 'asc' ? aVal - bVal : bVal - aVal
  })

  return filtered
})

const totalPages = computed(() =>
  Math.max(1, Math.ceil(filteredMasternodes.value.length / pageSize))
)

const pageStart = computed(() => (currentPage.value - 1) * pageSize)
const pageEnd = computed(() =>
  Math.min(pageStart.value + pageSize, filteredMasternodes.value.length)
)

const paginatedMasternodes = computed(() =>
  filteredMasternodes.value.slice(pageStart.value, pageEnd.value)
)

const sortBy = (field) => {
  if (sortField.value === field) {
    sortDirection.value = sortDirection.value === 'asc' ? 'desc' : 'asc'
  } else {
    sortField.value = field
    sortDirection.value = 'asc'
  }
}

const getSortIcon = (field) => {
  if (sortField.value !== field) return '↕'
  return sortDirection.value === 'asc' ? '↑' : '↓'
}

const goToMasternode = (mn) => {
  router.push(`/masternode/${mn.txhash}-${mn.outidx}`)
}

// Reset to page 1 when filters change
watch([statusFilter, networkFilter, searchQuery], () => {
  currentPage.value = 1
})

const fetchMasternodes = async () => {
  loading.value = true
  error.value = ''

  try {
    const [countData, listData] = await Promise.all([
      masternodeService.getMasternodeCount(),
      masternodeService.getMasternodeList()
    ])

    mnCount.value = countData
    masternodes.value = Array.isArray(listData) ? listData : []
  } catch (err) {
    error.value = 'Failed to load the masternode list.'
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
  animation: fadeIn 0.3s ease;
}

.masternode-list-page h1 {
  font-size: var(--text-4xl);
  font-weight: var(--weight-extrabold);
  margin-bottom: var(--space-2);
  color: var(--text-primary);
}

.page-subtitle {
  color: var(--text-secondary);
  margin-bottom: var(--space-8);
}

.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
  gap: var(--space-4);
  margin-bottom: var(--space-8);
}

.state-container {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--space-4);
  min-height: 300px;
  color: var(--text-tertiary);
}

.error-state h2 {
  color: var(--text-primary);
}

.error-state p {
  color: var(--danger);
}

.filters-card {
  margin-bottom: var(--space-6);
}

.filters {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: var(--space-4);
}

.filter-group {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.filter-group label {
  font-size: var(--text-xs);
  color: var(--text-secondary);
  text-transform: uppercase;
  font-weight: var(--weight-bold);
  letter-spacing: 0.05em;
}

.filter-select,
.filter-input {
  padding: var(--space-3);
  background: var(--bg-tertiary);
  border: 2px solid var(--border-secondary);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-family: var(--font-primary);
  font-size: var(--text-sm);
}

.filter-select:focus,
.filter-input:focus {
  outline: none;
  border-color: var(--border-accent);
}

.table-container {
  overflow-x: auto;
}

.mn-table {
  width: 100%;
  border-collapse: collapse;
}

.mn-table th {
  padding: var(--space-3) var(--space-4);
  text-align: left;
  font-size: var(--text-xs);
  font-weight: var(--weight-bold);
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.05em;
  border-bottom: 2px solid var(--border-secondary);
  background: var(--bg-tertiary);
  white-space: nowrap;
}

.mn-table th.sortable {
  cursor: pointer;
  user-select: none;
}

.mn-table th.sortable:hover {
  color: var(--text-accent);
}

.sort-icon {
  margin-left: var(--space-1);
  color: var(--text-tertiary);
}

.mn-table td {
  padding: var(--space-3) var(--space-4);
  font-size: var(--text-sm);
  border-bottom: 1px solid var(--border-subtle);
  white-space: nowrap;
}

.mn-row {
  cursor: pointer;
  transition: background var(--transition-fast);
}

.mn-row:hover {
  background: var(--bg-tertiary);
}

.rank {
  font-weight: var(--weight-bold);
  color: var(--text-accent);
}

.mono {
  font-family: var(--font-mono);
}

.address-link {
  color: var(--text-accent);
  text-decoration: none;
}

.address-link:hover {
  color: var(--pivx-accent-dark);
  text-decoration: underline;
}

.network {
  text-transform: uppercase;
  color: var(--text-secondary);
  font-size: var(--text-xs);
}

.protocol {
  font-family: var(--font-mono);
  color: var(--text-secondary);
}

.empty-note {
  text-align: center;
  color: var(--text-tertiary);
  padding: var(--space-8) 0;
}

.pagination {
  display: flex;
  justify-content: space-between;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-4);
  padding-top: var(--space-4);
  margin-top: var(--space-4);
  border-top: 1px solid var(--border-subtle);
}

.pagination-info {
  font-size: var(--text-sm);
  color: var(--text-tertiary);
}

.pagination-controls {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

.pagination-page {
  font-size: var(--text-sm);
  color: var(--text-secondary);
  font-weight: var(--weight-bold);
  white-space: nowrap;
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
  .stats-grid {
    grid-template-columns: repeat(2, 1fr);
  }

  .filters {
    grid-template-columns: 1fr;
  }

  .mn-table th,
  .mn-table td {
    padding: var(--space-2) var(--space-3);
    font-size: var(--text-xs);
  }
}
</style>
