<template>
  <AppLayout>
    <div class="governance-page">
      <div class="page-header">
        <h1>Governance</h1>
        <p class="page-subtitle">Budget proposals and community voting</p>
      </div>

      <!-- Loading State -->
      <div v-if="loading && proposals.length === 0" class="loading-container">
        <LoadingSpinner size="lg" text="Loading proposals..." />
      </div>

      <!-- Error State -->
      <div v-else-if="error" class="error-container">
        <EmptyState
          icon="⚠️"
          title="Failed to Load Proposals"
          :message="error"
        >
          <template #action>
            <Button @click="fetchProposals">Try Again</Button>
          </template>
        </EmptyState>
      </div>

      <!-- Proposals -->
      <div v-else>
        <!-- Budget Overview -->
        <Card class="budget-overview-card">
          <template #header>
            <div class="overview-header">
              <span class="header-icon">📊</span>
              <span>Budget Overview</span>
            </div>
          </template>

          <div class="budget-stats-grid">
            <div class="budget-stat">
              <div class="stat-label">Active Proposals</div>
              <div class="stat-value stat-accent">{{ passingProposals.length }}</div>
            </div>
            <div class="budget-stat">
              <div class="stat-label">Max Monthly Budget</div>
              <div class="stat-value" :title="maxMonthlyBudget + ' PIV'">{{ formatNumber(maxMonthlyBudget) }} PIV</div>
            </div>
            <div class="budget-stat">
              <div class="stat-label">Allocated (Approved)</div>
              <div class="stat-value stat-warning" :title="allocatedBudget + ' PIV'">{{ formatNumber(allocatedBudget) }} PIV</div>
            </div>
            <div class="budget-stat">
              <div class="stat-label">Remaining Budget</div>
              <div class="stat-value" :class="remainingBudget > 0 ? 'stat-success' : 'stat-danger'" :title="remainingBudget + ' PIV'">
                {{ formatNumber(remainingBudget) }} PIV
              </div>
            </div>
            <div class="budget-stat">
              <div class="stat-label">Next Payout</div>
              <div class="stat-value stat-info" :title="'Block ' + nextSuperblock">
                {{ timeUntilNextPayout }}
              </div>
            </div>
          </div>

          <!-- Budget Bar -->
          <div class="budget-bar-container">
            <div class="budget-bar">
              <div 
                class="budget-bar-fill" 
                :style="{ width: budgetUtilizationPercent + '%' }"
              ></div>
            </div>
            <div class="budget-bar-label">
              <span>{{ budgetUtilizationPercent.toFixed(1) }}% Allocated</span>
            </div>
          </div>
        </Card>

        <!-- Filter Tabs -->
        <div class="filter-tabs">
          <button
            :class="['filter-tab', { active: statusFilter === 'all' }]"
            @click="statusFilter = 'all'"
          >
            All ({{ proposals.length }})
          </button>
          <button
            :class="['filter-tab', { active: statusFilter === 'active' }]"
            @click="statusFilter = 'active'"
          >
            Active ({{ activeProposals.length }})
          </button>
          <button
            :class="['filter-tab', { active: statusFilter === 'passing' }]"
            @click="statusFilter = 'passing'"
          >
            Passing ({{ passingProposals.length }})
          </button>
          <button
            :class="['filter-tab', { active: statusFilter === 'failing' }]"
            @click="statusFilter = 'failing'"
          >
            Failing ({{ failingProposals.length }})
          </button>
        </div>

        <!-- Proposals Grid -->
        <div v-if="filteredProposals.length > 0" class="proposals-grid">
          <Card
            v-for="proposal in filteredProposals"
            :key="proposal.Hash"
            class="proposal-card"
            hover
            @click="viewProposal(proposal)"
          >
            <template #header>
              <div class="proposal-header">
                <h3 class="proposal-name">{{ proposal.Name }}</h3>
                <Badge :variant="getProposalDisplayInfo(proposal).variant" :title="getProposalDisplayInfo(proposal).explanation">
                  {{ getProposalDisplayInfo(proposal).label }}
                </Badge>
              </div>
            </template>

            <div class="proposal-body">
              <!-- Voting Stats -->
              <div class="vote-stats">
                <div class="vote-bar">
                  <div
                    class="vote-bar-fill yeas"
                    :style="{ width: getYeasPercentage(proposal) + '%' }"
                  ></div>
                  <div
                    class="vote-bar-fill nays"
                    :style="{ width: getNaysPercentage(proposal) + '%' }"
                  ></div>
                </div>
                <div class="vote-numbers">
                  <div class="vote-yeas">
                    <span class="vote-label">Yes:</span>
                    <span class="vote-value">{{ proposal.Yeas }}</span>
                  </div>
                  <div class="vote-nays">
                    <span class="vote-label">No:</span>
                    <span class="vote-value">{{ proposal.Nays }}</span>
                  </div>
                  <div class="vote-abstains">
                    <span class="vote-label">Abstain:</span>
                    <span class="vote-value">{{ proposal.Abstains }}</span>
                  </div>
                </div>
              </div>

              <!-- Payment Info -->
              <div class="payment-info">
                <InfoRow label="Monthly Payment">
                  <span class="payment-amount">{{ formatPIV(proposal.MonthlyPayment) }} PIV</span>
                </InfoRow>
                <InfoRow label="Total Payment">
                  <span class="payment-amount">{{ formatPIV(proposal.TotalPayment) }} PIV</span>
                </InfoRow>
                <InfoRow label="Payments Remaining">
                  {{ proposal.RemainingPaymentCount }} / {{ proposal.TotalPaymentCount }}
                </InfoRow>
                <InfoRow v-if="proposal.PaymentAddress" label="Payment Address">
                  <div class="payment-address-row" @click.stop>
                    <HashDisplay 
                      :hash="proposal.PaymentAddress" 
                      :truncate="true" 
                      show-copy
                    />
                  </div>
                </InfoRow>
              </div>

              <!-- Dates -->
              <div class="proposal-dates">
                <div class="date-item">
                  <span class="date-label">Start:</span>
                  <span class="date-value">Block {{ formatNumber(proposal.BlockStart) }}</span>
                </div>
                <div class="date-item">
                  <span class="date-label">End:</span>
                  <span class="date-value">Block {{ formatNumber(proposal.BlockEnd) }}</span>
                </div>
              </div>

              <!-- URL -->
              <div v-if="proposal.URL" class="proposal-url">
                <a :href="proposal.URL" target="_blank" class="external-link" @click.stop>
                  View Discussion →
                </a>
              </div>
            </div>
          </Card>
        </div>

        <!-- Empty State -->
        <EmptyState
          v-else
          icon="📭"
          title="No Proposals"
          message="No proposals match your filter"
        />
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { useChainStore } from '@/stores/chainStore'
import { governanceService } from '@/services/governanceService'
import { masternodeService } from '@/services/masternodeService'
import { formatNumber, formatPIV } from '@/utils/formatters'
import {
  calculateGovernanceStats,
  getProposalStatus,
  getStatusLabel,
  getStatusVariant,
  getStatusExplanation,
  ProposalStatus,
  PIVX_GOVERNANCE
} from '@/utils/governanceStatus'
import AppLayout from '@/components/layout/AppLayout.vue'
import Card from '@/components/common/Card.vue'
import Badge from '@/components/common/Badge.vue'
import Button from '@/components/common/Button.vue'
import InfoRow from '@/components/common/InfoRow.vue'
import HashDisplay from '@/components/common/HashDisplay.vue'
import LoadingSpinner from '@/components/common/LoadingSpinner.vue'
import EmptyState from '@/components/common/EmptyState.vue'

const router = useRouter()
const chainStore = useChainStore()

const proposals = ref([])
const mnCount = ref(null)
const loading = ref(false)
const error = ref('')
const statusFilter = ref('all')

// Current blockchain height (reactive)
const currentBlockHeight = computed(() => {
  return chainStore.syncHeight || chainStore.height || 0
})

// Calculate all governance statistics using PIVX Core rules
const governanceStats = computed(() => {
  if (!proposals.value.length || !mnCount.value || !currentBlockHeight.value) {
    return null
  }
  
  return calculateGovernanceStats(
    proposals.value,
    currentBlockHeight.value,
    mnCount.value.enabled
  )
})

// Computed properties for budget overview
const maxMonthlyBudget = computed(() => PIVX_GOVERNANCE.MAX_MONTHLY_BUDGET)

const passingProposals = computed(() => {
  return governanceStats.value?.fundedProposals || []
})

const passingUnfundedProposals = computed(() => {
  return governanceStats.value?.unfundedProposals || []
})

const allocatedBudget = computed(() => {
  return governanceStats.value?.budget.allocated || 0
})

const remainingBudget = computed(() => {
  return governanceStats.value?.budget.remaining || PIVX_GOVERNANCE.MAX_MONTHLY_BUDGET
})

const budgetUtilizationPercent = computed(() => {
  return governanceStats.value?.budget.utilization || 0
})

// Calculate next superblock and time until payout
const nextSuperblock = computed(() => {
  if (!currentBlockHeight.value) return 0
  const BLOCKS_PER_BUDGET_CYCLE = 43200
  return Math.ceil(currentBlockHeight.value / BLOCKS_PER_BUDGET_CYCLE) * BLOCKS_PER_BUDGET_CYCLE
})

const blocksUntilPayout = computed(() => {
  if (!currentBlockHeight.value) return 0
  return nextSuperblock.value - currentBlockHeight.value
})

const timeUntilNextPayout = computed(() => {
  if (!blocksUntilPayout.value) return '—'
  
  const SECONDS_PER_BLOCK = 60
  const totalSeconds = blocksUntilPayout.value * SECONDS_PER_BLOCK
  
  const days = Math.floor(totalSeconds / 86400)
  const hours = Math.floor((totalSeconds % 86400) / 3600)
  const minutes = Math.floor((totalSeconds % 3600) / 60)
  
  if (days > 0) {
    return `${days}d ${hours}h`
  } else if (hours > 0) {
    return `${hours}h ${minutes}m`
  } else {
    return `${minutes}m`
  }
})

// Active proposals (not completed, valid, and still has remaining payments)
const activeProposals = computed(() => {
  if (!proposals.value.length || !currentBlockHeight.value || !governanceStats.value) {
    return []
  }
  return proposals.value.filter(p => {
    const status = getProposalStatusForProposal(p)
    return status !== ProposalStatus.COMPLETED && 
           status !== ProposalStatus.INVALID &&
           (p.RemainingPaymentCount || 0) > 0 // Only show proposals with remaining payments
  })
})

// Failing proposals (active but not meeting threshold, and has remaining payments)
const failingProposals = computed(() => {
  if (!proposals.value.length || !currentBlockHeight.value || !governanceStats.value) {
    return []
  }
  return proposals.value.filter(p => {
    return getProposalStatusForProposal(p) === ProposalStatus.FAILING &&
           (p.RemainingPaymentCount || 0) > 0 // Only show proposals with remaining payments
  })
})

// Get proposal status with funding information
const getProposalStatusForProposal = (proposal) => {
  if (!governanceStats.value) return ProposalStatus.ACTIVE
  
  const isFunded = governanceStats.value.fundedProposals.some(p => p.Hash === proposal.Hash)
  
  return getProposalStatus(
    proposal,
    currentBlockHeight.value,
    governanceStats.value.voting.threshold,
    isFunded
  )
}

const filteredProposals = computed(() => {
  // Apply status filter
  let filtered
  switch (statusFilter.value) {
    case 'active':
      filtered = activeProposals.value
      break
    case 'passing':
      filtered = passingProposals.value
      break
    case 'failing':
      filtered = failingProposals.value
      break
    default:
      // 'all' - show all proposals with remaining payments
      filtered = proposals.value.filter(p => (p.RemainingPaymentCount || 0) > 0)
      break
  }
  return filtered
})

// Get display information for a proposal
const getProposalDisplayInfo = (proposal) => {
  const status = getProposalStatusForProposal(proposal)
  return {
    status,
    label: getStatusLabel(status),
    variant: getStatusVariant(status),
    explanation: getStatusExplanation(status, proposal),
  }
}

const getYeasPercentage = (proposal) => {
  const total = proposal.Yeas + proposal.Nays
  if (total === 0) return 0
  return (proposal.Yeas / total) * 100
}

const getNaysPercentage = (proposal) => {
  const total = proposal.Yeas + proposal.Nays
  if (total === 0) return 0
  return (proposal.Nays / total) * 100
}

const viewProposal = (proposal) => {
  router.push(`/governance/${encodeURIComponent(proposal.Name)}`)
}

const fetchProposals = async () => {
  loading.value = true
  error.value = ''

  try {
    // Fetch proposals, masternode count, and chain state in parallel
    const [proposalsData, mnCountData] = await Promise.all([
      governanceService.getBudgetInfo(),
      masternodeService.getMasternodeCount(),
      chainStore.fetchChainState()
    ])
    
    proposals.value = proposalsData
    mnCount.value = mnCountData
  } catch (err) {
    console.error('Failed to fetch proposals:', err)
    error.value = err.message || 'Failed to load proposals'
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
  padding: var(--space-6);
  max-width: 1400px;
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

.budget-overview-card {
  margin-bottom: var(--space-6);
  background: linear-gradient(135deg, rgba(102, 45, 145, 0.1) 0%, rgba(42, 27, 66, 0.3) 100%);
  border: 2px solid rgba(89, 252, 179, 0.2);
}

.overview-header {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--text-lg);
  font-weight: var(--weight-bold);
}

.header-icon {
  font-size: var(--text-xl);
}

.budget-stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: var(--space-4);
  margin-bottom: var(--space-5);
}

.budget-stat {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  padding: var(--space-5);
  background: linear-gradient(135deg, rgba(59, 44, 84, 0.6) 0%, rgba(45, 34, 64, 0.8) 100%);
  border-radius: var(--radius-lg);
  border: 1px solid rgba(89, 252, 179, 0.15);
  transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1);
  min-width: 0;
  overflow: hidden;
  align-items: flex-start;
  position: relative;
  backdrop-filter: blur(10px);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.2);
}

.budget-stat::before {
  content: '';
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 3px;
  background: linear-gradient(90deg, var(--pivx-accent) 0%, rgba(89, 252, 179, 0.3) 100%);
  opacity: 0;
  transition: opacity 0.3s ease;
}

.budget-stat:hover {
  transform: translateY(-4px) scale(1.02);
  box-shadow: 0 12px 24px rgba(89, 252, 179, 0.15), 
              0 0 40px rgba(89, 252, 179, 0.1);
  border-color: rgba(89, 252, 179, 0.4);
}

.budget-stat:hover::before {
  opacity: 1;
}

.stat-label {
  font-size: 0.65rem;
  color: rgba(255, 255, 255, 0.6);
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 1px;
  line-height: 1.2;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  width: 100%;
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.3);
}

.stat-value {
  font-size: 1.35rem;
  font-weight: 700;
  color: var(--text-primary);
  font-family: var(--font-mono);
  line-height: 1.1;
  width: 100%;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  text-shadow: 0 2px 4px rgba(0, 0, 0, 0.4);
}

.stat-accent {
  color: var(--pivx-accent);
  text-shadow: 0 0 20px rgba(89, 252, 179, 0.5),
               0 2px 4px rgba(0, 0, 0, 0.4);
}

.stat-info {
  color: #59b3fc;
  text-shadow: 0 0 15px rgba(89, 179, 252, 0.4),
               0 2px 4px rgba(0, 0, 0, 0.4);
}

.stat-success {
  color: #5dfc8a;
  text-shadow: 0 0 15px rgba(93, 252, 138, 0.4),
               0 2px 4px rgba(0, 0, 0, 0.4);
}

.stat-warning {
  color: #fcb559;
  text-shadow: 0 0 15px rgba(252, 181, 89, 0.4),
               0 2px 4px rgba(0, 0, 0, 0.4);
}

.stat-danger {
  color: #fc5959;
  text-shadow: 0 0 15px rgba(252, 89, 89, 0.4),
               0 2px 4px rgba(0, 0, 0, 0.4);
}

.budget-bar-container {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.budget-bar {
  height: 12px;
  background: var(--bg-tertiary);
  border-radius: var(--radius-md);
  overflow: hidden;
  border: 1px solid var(--border-secondary);
}

.budget-bar-fill {
  height: 100%;
  background: linear-gradient(90deg, var(--pivx-purple-primary) 0%, var(--pivx-accent) 100%);
  transition: width 0.5s ease-out;
  box-shadow: 0 0 10px rgba(89, 252, 179, 0.5);
}

.budget-bar-label {
  display: flex;
  justify-content: center;
  font-size: var(--text-sm);
  color: var(--text-secondary);
  font-weight: var(--weight-bold);
}

.filter-tabs {
  display: flex;
  gap: var(--space-2);
  margin-bottom: var(--space-6);
  border-bottom: 2px solid var(--border-subtle);
  overflow-x: auto;
}

.filter-tab {
  padding: var(--space-3) var(--space-4);
  background: none;
  border: none;
  color: var(--text-secondary);
  font-family: var(--font-primary);
  font-size: var(--text-base);
  font-weight: 600;
  cursor: pointer;
  border-bottom: 3px solid transparent;
  margin-bottom: -2px;
  transition: all 0.2s;
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
  grid-template-columns: repeat(auto-fill, minmax(400px, 1fr));
  gap: var(--space-4);
}

.proposal-card {
  cursor: pointer;
}

.proposal-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-3);
}

.proposal-name {
  font-size: var(--text-lg);
  font-weight: 700;
  color: var(--text-primary);
  margin: 0;
  flex: 1;
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
  transition: width 0.3s;
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
}

.vote-yeas,
.vote-nays,
.vote-abstains {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.vote-label {
  color: var(--text-secondary);
}

.vote-value {
  font-weight: 700;
  color: var(--text-primary);
}

.payment-info {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: var(--space-3);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
}

.payment-amount {
  font-family: var(--font-mono);
  font-weight: 700;
  color: var(--text-accent);
}

.payment-address-row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
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
  font-weight: 600;
}

.date-value {
  color: var(--text-primary);
  font-family: var(--font-mono);
}

.proposal-url {
  margin-top: var(--space-2);
}

.external-link {
  color: var(--text-accent);
  text-decoration: none;
  font-size: var(--text-sm);
  font-weight: 600;
  transition: opacity 0.2s;
}

.external-link:hover {
  opacity: 0.8;
}

.loading-container,
.error-container {
  min-height: 400px;
  display: flex;
  align-items: center;
  justify-content: center;
}

@media (max-width: 768px) {
  .governance-page {
    padding: var(--space-4);
  }

  .budget-stats-grid {
    grid-template-columns: repeat(2, 1fr);
    gap: var(--space-3);
  }

  .stat-label {
    font-size: var(--text-xs);
  }

  .stat-value {
    font-size: var(--text-xl);
  }

  .proposals-grid {
    grid-template-columns: 1fr;
  }

  .filter-tabs {
    padding-bottom: var(--space-2);
  }

  .filter-tab {
    font-size: var(--text-sm);
    padding: var(--space-2) var(--space-3);
  }
}
</style>
