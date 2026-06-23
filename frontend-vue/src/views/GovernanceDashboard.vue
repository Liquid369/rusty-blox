<template>
  <AppLayout>
    <div class="governance-page">
      <h1>Governance</h1>
      <p class="page-subtitle">Budget proposals and community voting</p>

      <!-- Loading -->
      <div v-if="loading" class="state-container">
        <span class="loading-spinner"></span>
        <p>Loading proposals...</p>
      </div>

      <!-- Error -->
      <div v-else-if="error" class="state-container error-state">
        <h2>Failed to load proposals</h2>
        <p>{{ error }}</p>
        <UiButton @click="fetchProposals">Try Again</UiButton>
      </div>

      <template v-else>
        <!-- Budget Overview -->
        <div class="stats-grid">
          <StatCard label="Active Proposals" :value="activeProposals.length" format="number" />
          <StatCard
            label="Max Monthly Budget"
            :value="formatNumber(MAX_MONTHLY_BUDGET)"
            subtitle="PIV"
          />
          <StatCard label="Allocated" :value="formatNumber(allocatedBudget)" subtitle="PIV" />
          <StatCard
            label="Remaining Budget"
            :value="formatNumber(remainingBudget)"
            subtitle="PIV"
          />
          <StatCard
            label="Next Payout"
            :value="timeUntilNextPayout"
            :subtitle="nextSuperblock ? `Block ${nextSuperblock.toLocaleString()}` : ''"
          />
        </div>

        <!-- Budget Utilization Bar -->
        <UiCard class="budget-bar-card">
          <div class="budget-bar">
            <div class="budget-bar-fill" :style="{ width: budgetUtilization + '%' }"></div>
          </div>
          <p class="budget-bar-label">{{ budgetUtilization.toFixed(1) }}% of monthly budget allocated</p>
        </UiCard>

        <!-- Filter Tabs -->
        <div class="filter-tabs">
          <button
            v-for="tab in filterTabs"
            :key="tab.value"
            :class="['filter-tab', { active: statusFilter === tab.value }]"
            @click="statusFilter = tab.value"
          >
            {{ tab.label }} ({{ tab.count }})
          </button>
        </div>

        <!-- Proposals Grid -->
        <div v-if="filteredProposals.length > 0" class="proposals-grid">
          <UiCard
            v-for="proposal in filteredProposals"
            :key="proposal.Hash"
            hover
            clickable
            @click="viewProposal(proposal)"
          >
            <template #header>
              <div class="proposal-header">
                <h3 class="proposal-name">{{ proposal.Name }}</h3>
                <span
                  :class="['badge', statusBadgeClass(proposalStatus(proposal))]"
                  :title="statusLabel(proposalStatus(proposal))"
                >
                  {{ statusLabel(proposalStatus(proposal)) }}
                </span>
              </div>
            </template>

            <div class="proposal-body">
              <!-- Vote Bar -->
              <div class="vote-stats">
                <div class="vote-bar">
                  <div class="vote-bar-fill yeas" :style="{ width: yeasPercent(proposal) + '%' }"></div>
                  <div class="vote-bar-fill nays" :style="{ width: naysPercent(proposal) + '%' }"></div>
                </div>
                <div class="vote-numbers">
                  <span><span class="vote-label">Yes:</span> <strong>{{ proposal.Yeas }}</strong></span>
                  <span><span class="vote-label">No:</span> <strong>{{ proposal.Nays }}</strong></span>
                  <span><span class="vote-label">Abstain:</span> <strong>{{ proposal.Abstains }}</strong></span>
                </div>
              </div>

              <!-- Payment Info -->
              <div class="payment-info">
                <div class="payment-row">
                  <span class="payment-label">Monthly Payment</span>
                  <span class="payment-amount">{{ formatNumber(proposal.MonthlyPayment) }} PIV</span>
                </div>
                <div class="payment-row">
                  <span class="payment-label">Total Payment</span>
                  <span class="payment-amount">{{ formatNumber(proposal.TotalPayment) }} PIV</span>
                </div>
                <div class="payment-row">
                  <span class="payment-label">Payments Remaining</span>
                  <span>{{ proposal.RemainingPaymentCount }} / {{ proposal.TotalPaymentCount }}</span>
                </div>
              </div>

              <!-- Block Window -->
              <div class="proposal-dates">
                <div class="date-item">
                  <span class="date-label">Start</span>
                  <span class="date-value">Block {{ formatNumber(proposal.BlockStart) }}</span>
                </div>
                <div class="date-item">
                  <span class="date-label">End</span>
                  <span class="date-value">Block {{ formatNumber(proposal.BlockEnd) }}</span>
                </div>
              </div>

              <!-- Discussion link -->
              <a
                v-if="proposal.URL"
                :href="proposal.URL"
                target="_blank"
                rel="noopener noreferrer"
                class="external-link"
                @click.stop
              >
                View Discussion →
              </a>
            </div>
          </UiCard>
        </div>

        <!-- Empty -->
        <div v-else class="state-container">
          <h2>No proposals</h2>
          <p>No proposals match your filter.</p>
        </div>
      </template>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { governanceService } from '@/services/governanceService'
import { masternodeService } from '@/services/masternodeService'
import { chainService } from '@/services'
import AppLayout from '@/components/layout/AppLayout.vue'
import StatCard from '@/components/common/StatCard.vue'
import UiCard from '@/components/common/UiCard.vue'
import UiButton from '@/components/common/UiButton.vue'

// PIVX governance constants
const MAX_MONTHLY_BUDGET = 432000
const BLOCKS_PER_BUDGET_CYCLE = 43200
const SECONDS_PER_BLOCK = 60
const PASSING_THRESHOLD_PERCENT = 0.1

const router = useRouter()

const proposals = ref([])
const projection = ref([])
const mnCount = ref(null)
const chainHeight = ref(0)
const loading = ref(true)
const error = ref('')
const statusFilter = ref('all')

const formatNumber = (value) => Number(value || 0).toLocaleString(undefined, { maximumFractionDigits: 0 })

// 10% of enabled masternodes (net votes) needed to pass
const passingThreshold = computed(() => {
  if (!mnCount.value?.enabled) return 0
  return Math.ceil(mnCount.value.enabled * PASSING_THRESHOLD_PERCENT)
})

// Proposals projected to be funded in the next superblock
const fundedHashes = computed(() => {
  return new Set(projection.value.filter((p) => (p.Allotted || 0) > 0).map((p) => p.Hash))
})

const allocatedBudget = computed(() =>
  projection.value.reduce((sum, p) => sum + (p.Allotted || 0), 0)
)

const remainingBudget = computed(() => Math.max(MAX_MONTHLY_BUDGET - allocatedBudget.value, 0))

const budgetUtilization = computed(() =>
  Math.min((allocatedBudget.value / MAX_MONTHLY_BUDGET) * 100, 100)
)

const nextSuperblock = computed(() => {
  if (!chainHeight.value) return 0
  return Math.ceil(chainHeight.value / BLOCKS_PER_BUDGET_CYCLE) * BLOCKS_PER_BUDGET_CYCLE
})

const timeUntilNextPayout = computed(() => {
  if (!chainHeight.value) return '—'
  const blocksLeft = nextSuperblock.value - chainHeight.value
  const totalSeconds = blocksLeft * SECONDS_PER_BLOCK

  const days = Math.floor(totalSeconds / 86400)
  const hours = Math.floor((totalSeconds % 86400) / 3600)
  const minutes = Math.floor((totalSeconds % 3600) / 60)

  if (days > 0) return `${days}d ${hours}h`
  if (hours > 0) return `${hours}h ${minutes}m`
  return `${minutes}m`
})

const isCompleted = (proposal) => {
  return chainHeight.value > 0 && chainHeight.value >= proposal.BlockEnd
}

const netVotes = (proposal) => proposal.Yeas - proposal.Nays

const proposalStatus = (proposal) => {
  if (!proposal.IsValid) return 'invalid'
  if (isCompleted(proposal)) return 'completed'
  if (fundedHashes.value.has(proposal.Hash)) return 'passing'
  if (netVotes(proposal) >= passingThreshold.value && passingThreshold.value > 0) {
    return 'passing_unfunded'
  }
  return 'failing'
}

const statusLabel = (status) => {
  const labels = {
    passing: 'Passing',
    passing_unfunded: 'Passing (Unfunded)',
    failing: 'Failing',
    completed: 'Completed',
    invalid: 'Invalid'
  }
  return labels[status] || 'Active'
}

const statusBadgeClass = (status) => {
  const classes = {
    passing: 'badge-success',
    passing_unfunded: 'badge-warning',
    failing: 'badge-danger',
    completed: 'badge-info',
    invalid: 'badge-danger'
  }
  return classes[status] || 'badge-info'
}

// Proposals still in their lifecycle with payments remaining
const activeProposals = computed(() =>
  proposals.value.filter((p) => {
    const status = proposalStatus(p)
    return status !== 'completed' && status !== 'invalid' && (p.RemainingPaymentCount || 0) > 0
  })
)

const passingProposals = computed(() =>
  activeProposals.value.filter((p) => proposalStatus(p) === 'passing')
)

const failingProposals = computed(() =>
  activeProposals.value.filter((p) => proposalStatus(p) === 'failing')
)

const filterTabs = computed(() => [
  { value: 'all', label: 'All', count: proposals.value.length },
  { value: 'active', label: 'Active', count: activeProposals.value.length },
  { value: 'passing', label: 'Passing', count: passingProposals.value.length },
  { value: 'failing', label: 'Failing', count: failingProposals.value.length }
])

const filteredProposals = computed(() => {
  switch (statusFilter.value) {
    case 'active':
      return activeProposals.value
    case 'passing':
      return passingProposals.value
    case 'failing':
      return failingProposals.value
    default:
      return proposals.value
  }
})

const yeasPercent = (proposal) => {
  const total = proposal.Yeas + proposal.Nays
  return total === 0 ? 0 : (proposal.Yeas / total) * 100
}

const naysPercent = (proposal) => {
  const total = proposal.Yeas + proposal.Nays
  return total === 0 ? 0 : (proposal.Nays / total) * 100
}

const viewProposal = (proposal) => {
  router.push(`/proposal/${encodeURIComponent(proposal.Name)}`)
}

const fetchProposals = async () => {
  loading.value = true
  error.value = ''

  try {
    const [proposalsData, projectionData, mnCountData, status] = await Promise.all([
      governanceService.getBudgetInfo(),
      governanceService.getBudgetProjection().catch(() => []),
      masternodeService.getMasternodeCount(),
      chainService.getStatus()
    ])

    proposals.value = Array.isArray(proposalsData) ? proposalsData : []
    projection.value = Array.isArray(projectionData) ? projectionData : []
    mnCount.value = mnCountData
    chainHeight.value = status?.height || 0
  } catch (err) {
    error.value = 'Failed to load governance data.'
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  fetchProposals()
})
</script>

<style scoped>
.governance-page {
  animation: fadeIn 0.3s ease;
}

.governance-page h1 {
  font-size: var(--text-4xl);
  font-weight: var(--weight-extrabold);
  margin-bottom: var(--space-2);
  color: var(--text-primary);
}

.page-subtitle {
  color: var(--text-secondary);
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
  text-align: center;
}

.state-container h2 {
  color: var(--text-primary);
}

.error-state p {
  color: var(--danger);
}

.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(190px, 1fr));
  gap: var(--space-4);
  margin-bottom: var(--space-6);
}

.budget-bar-card {
  margin-bottom: var(--space-8);
}

.budget-bar {
  height: 12px;
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  overflow: hidden;
  border: 1px solid var(--border-secondary);
}

.budget-bar-fill {
  height: 100%;
  background: linear-gradient(90deg, var(--pivx-purple-primary) 0%, var(--pivx-accent) 100%);
  transition: width 0.5s ease-out;
}

.budget-bar-label {
  margin: var(--space-3) 0 0;
  text-align: center;
  font-size: var(--text-sm);
  font-weight: var(--weight-bold);
  color: var(--text-secondary);
}

.filter-tabs {
  display: flex;
  gap: var(--space-2);
  margin-bottom: var(--space-6);
  border-bottom: 2px solid var(--border-primary);
  overflow-x: auto;
}

.filter-tab {
  padding: var(--space-3) var(--space-4);
  background: none;
  border: none;
  color: var(--text-secondary);
  font-family: var(--font-primary);
  font-size: var(--text-base);
  font-weight: var(--weight-bold);
  cursor: pointer;
  border-bottom: 3px solid transparent;
  margin-bottom: -2px;
  transition: all var(--transition-fast);
  white-space: nowrap;
}

.filter-tab:hover {
  color: var(--text-primary);
}

.filter-tab.active {
  color: var(--text-accent);
  border-bottom-color: var(--border-accent);
}

.proposals-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(380px, 1fr));
  gap: var(--space-4);
}

.proposal-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-3);
}

.proposal-name {
  font-size: var(--text-lg);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
  margin: 0;
  flex: 1;
  word-break: break-word;
}

.proposal-body {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.vote-stats {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.vote-bar {
  height: 8px;
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  overflow: hidden;
  display: flex;
}

.vote-bar-fill {
  height: 100%;
  transition: width var(--transition-slow);
}

.vote-bar-fill.yeas {
  background: var(--success);
}

.vote-bar-fill.nays {
  background: var(--danger);
}

.vote-numbers {
  display: flex;
  justify-content: space-between;
  font-size: var(--text-sm);
  color: var(--text-primary);
}

.vote-label {
  color: var(--text-secondary);
}

.payment-info {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: var(--space-3);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  font-size: var(--text-sm);
}

.payment-row {
  display: flex;
  justify-content: space-between;
  gap: var(--space-4);
}

.payment-label {
  color: var(--text-secondary);
}

.payment-amount {
  font-family: var(--font-mono);
  font-weight: var(--weight-bold);
  color: var(--text-accent);
}

.proposal-dates {
  display: flex;
  justify-content: space-between;
  font-size: var(--text-sm);
}

.date-item {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.date-label {
  color: var(--text-secondary);
  text-transform: uppercase;
  font-size: var(--text-xs);
  font-weight: var(--weight-bold);
}

.date-value {
  color: var(--text-primary);
  font-family: var(--font-mono);
}

.external-link {
  color: var(--text-accent);
  text-decoration: none;
  font-size: var(--text-sm);
  font-weight: var(--weight-bold);
  transition: color var(--transition-fast);
}

.external-link:hover {
  color: var(--pivx-accent-dark);
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

  .proposals-grid {
    grid-template-columns: 1fr;
  }

  .filter-tab {
    font-size: var(--text-sm);
    padding: var(--space-2) var(--space-3);
  }
}
</style>
