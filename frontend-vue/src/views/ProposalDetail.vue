<template>
  <AppLayout>
    <div class="proposal-detail-page">
      <!-- Loading -->
      <div v-if="loading" class="state-container">
        <span class="loading-spinner"></span>
        <p>Loading proposal...</p>
      </div>

      <!-- Error -->
      <div v-else-if="error" class="state-container error-state">
        <h2>Proposal not found</h2>
        <p>{{ error }}</p>
        <UiButton @click="$router.push('/governance')">View All Proposals</UiButton>
      </div>

      <div v-else-if="proposal">
        <router-link to="/governance" class="back-link">← Back to Governance</router-link>

        <div class="header-title">
          <h1>{{ proposal.Name }}</h1>
          <span :class="['badge', statusBadgeClass]">{{ statusLabel }}</span>
        </div>

        <!-- Proposal Information -->
        <UiCard class="mt-6">
          <template #header>
            <h2>Proposal Information</h2>
          </template>

          <div class="detail-grid">
            <div class="detail-row">
              <span class="detail-label">Proposal Hash</span>
              <span class="detail-value mono">{{ proposal.Hash }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Fee Hash</span>
              <span class="detail-value mono">{{ proposal.FeeHash }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Payment Address</span>
              <router-link
                :to="`/address/${proposal.PaymentAddress}`"
                class="detail-value mono link"
              >
                {{ proposal.PaymentAddress }}
              </router-link>
            </div>
            <div class="detail-row">
              <span class="detail-label">Forum URL</span>
              <a
                :href="proposal.URL"
                target="_blank"
                rel="noopener noreferrer"
                class="detail-value link"
              >
                {{ proposal.URL }} →
              </a>
            </div>
            <div class="detail-row">
              <span class="detail-label">Start Block</span>
              <span class="detail-value">{{ formatNumber(proposal.BlockStart) }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">End Block</span>
              <span class="detail-value">{{ formatNumber(proposal.BlockEnd) }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Total Payments</span>
              <span class="detail-value">{{ proposal.TotalPaymentCount }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Remaining Payments</span>
              <span class="detail-value">{{ proposal.RemainingPaymentCount }}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Established</span>
              <span class="detail-value">
                <span :class="['badge', proposal.IsEstablished ? 'badge-success' : 'badge-warning']">
                  {{ proposal.IsEstablished ? 'Yes' : 'No' }}
                </span>
              </span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Valid</span>
              <span class="detail-value">
                <span :class="['badge', proposal.IsValid ? 'badge-success' : 'badge-danger']">
                  {{ proposal.IsValid ? 'Yes' : 'No' }}
                </span>
              </span>
            </div>
          </div>
        </UiCard>

        <!-- Payment + Voting -->
        <div class="two-column-grid mt-6">
          <UiCard>
            <template #header>
              <h2>Payment Details</h2>
            </template>
            <div class="payment-details">
              <div class="payment-item">
                <span class="payment-label">Monthly Payment</span>
                <span class="payment-value">{{ formatNumber(proposal.MonthlyPayment) }} PIV</span>
              </div>
              <div class="payment-item">
                <span class="payment-label">Total Payment</span>
                <span class="payment-value">{{ formatNumber(proposal.TotalPayment) }} PIV</span>
              </div>
              <div class="payment-item">
                <span class="payment-label">Allotted</span>
                <span class="payment-value">{{ formatNumber(proposal.Allotted) }} PIV</span>
              </div>
              <div class="payment-item">
                <span class="payment-label">Approval Ratio</span>
                <span class="payment-value">{{ ((proposal.Ratio || 0) * 100).toFixed(1) }}%</span>
              </div>
            </div>
          </UiCard>

          <UiCard>
            <template #header>
              <h2>Voting Statistics</h2>
            </template>
            <div class="voting-stats">
              <div class="vote-bar">
                <div class="vote-bar-fill yeas" :style="{ width: yeasPercent + '%' }"></div>
                <div class="vote-bar-fill nays" :style="{ width: naysPercent + '%' }"></div>
              </div>

              <div class="vote-summary">
                <div class="vote-item">
                  <span class="vote-label">Yes Votes</span>
                  <span class="vote-value yes">{{ formatNumber(proposal.Yeas) }}</span>
                  <span class="vote-percentage">{{ yeasPercent.toFixed(1) }}%</span>
                </div>
                <div class="vote-item">
                  <span class="vote-label">No Votes</span>
                  <span class="vote-value no">{{ formatNumber(proposal.Nays) }}</span>
                  <span class="vote-percentage">{{ naysPercent.toFixed(1) }}%</span>
                </div>
                <div class="vote-item">
                  <span class="vote-label">Abstain</span>
                  <span class="vote-value">{{ formatNumber(proposal.Abstains) }}</span>
                </div>
              </div>

              <div class="net-votes">
                <span class="net-label">Net Votes</span>
                <span :class="['net-value', { positive: netVotes > 0, negative: netVotes < 0 }]">
                  {{ netVotes > 0 ? '+' : '' }}{{ formatNumber(netVotes) }}
                </span>
              </div>

              <div v-if="passingThreshold > 0" class="threshold-info">
                <div class="threshold-item">
                  <span class="threshold-label">Required (10% of MNs)</span>
                  <span class="threshold-value">{{ formatNumber(passingThreshold) }}</span>
                </div>
                <span :class="['badge', meetsThreshold ? 'badge-success' : 'badge-warning']">
                  {{ meetsThreshold ? 'Meeting Threshold' : 'Below Threshold' }}
                </span>
              </div>
            </div>
          </UiCard>
        </div>

        <!-- Individual Votes -->
        <UiCard v-if="votes.length > 0" class="mt-6">
          <template #header>
            <div class="votes-header">
              <h2>Individual Votes</h2>
              <span class="badge badge-info">{{ votes.length }} masternodes</span>
            </div>
          </template>

          <div class="votes-list">
            <div v-for="vote in votes" :key="vote.nHash" class="vote-entry">
              <span class="vote-mn mono">{{ truncateMiddle(vote.mnId) }}</span>
              <span class="vote-time">{{ formatVoteTime(vote.nTime) }}</span>
              <span :class="['badge', voteBadgeClass(vote.Vote)]">{{ vote.Vote }}</span>
            </div>
          </div>
        </UiCard>
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { useRoute } from 'vue-router'
import { governanceService } from '@/services/governanceService'
import { masternodeService } from '@/services/masternodeService'
import { chainService } from '@/services'
import AppLayout from '@/components/layout/AppLayout.vue'
import UiCard from '@/components/common/UiCard.vue'
import UiButton from '@/components/common/UiButton.vue'

const PASSING_THRESHOLD_PERCENT = 0.1

const route = useRoute()

const proposal = ref(null)
const votes = ref([])
const mnCount = ref(null)
const chainHeight = ref(0)
const loading = ref(true)
const error = ref('')

const formatNumber = (value) => Number(value || 0).toLocaleString(undefined, { maximumFractionDigits: 0 })

const truncateMiddle = (value, start = 12, end = 8) => {
  if (!value) return ''
  if (value.length <= start + end + 3) return value
  return `${value.slice(0, start)}...${value.slice(-end)}`
}

const formatVoteTime = (seconds) => {
  if (!seconds) return ''
  return new Date(seconds * 1000).toLocaleDateString()
}

const netVotes = computed(() => {
  if (!proposal.value) return 0
  return proposal.value.Yeas - proposal.value.Nays
})

const passingThreshold = computed(() => {
  if (!mnCount.value?.enabled) return 0
  return Math.ceil(mnCount.value.enabled * PASSING_THRESHOLD_PERCENT)
})

const meetsThreshold = computed(() => netVotes.value >= passingThreshold.value)

const isCompleted = computed(() => {
  if (!proposal.value || !chainHeight.value) return false
  return chainHeight.value >= proposal.value.BlockEnd
})

const statusLabel = computed(() => {
  if (!proposal.value) return ''
  if (!proposal.value.IsValid) return 'Invalid'
  if (isCompleted.value) return 'Completed'
  return meetsThreshold.value ? 'Passing' : 'Failing'
})

const statusBadgeClass = computed(() => {
  if (!proposal.value) return 'badge-info'
  if (!proposal.value.IsValid) return 'badge-danger'
  if (isCompleted.value) return 'badge-info'
  return meetsThreshold.value ? 'badge-success' : 'badge-warning'
})

const yeasPercent = computed(() => {
  if (!proposal.value) return 0
  const total = proposal.value.Yeas + proposal.value.Nays
  return total === 0 ? 0 : (proposal.value.Yeas / total) * 100
})

const naysPercent = computed(() => {
  if (!proposal.value) return 0
  const total = proposal.value.Yeas + proposal.value.Nays
  return total === 0 ? 0 : (proposal.value.Nays / total) * 100
})

const voteBadgeClass = (vote) => {
  if (vote === 'YES') return 'badge-success'
  if (vote === 'NO') return 'badge-danger'
  return 'badge-info'
}

const fetchProposal = async (name) => {
  loading.value = true
  error.value = ''
  proposal.value = null
  votes.value = []

  try {
    const decodedName = decodeURIComponent(name)

    const [budgetInfo, mnCountData, status] = await Promise.all([
      governanceService.getBudgetInfo(),
      masternodeService.getMasternodeCount().catch(() => null),
      chainService.getStatus().catch(() => null)
    ])

    mnCount.value = mnCountData
    chainHeight.value = status?.height || 0

    const found = Array.isArray(budgetInfo)
      ? budgetInfo.find((p) => p.Name === decodedName)
      : null

    if (!found) {
      error.value = `No proposal named "${decodedName}" was found.`
      return
    }

    proposal.value = found

    // Votes are non-critical; continue without them on failure
    try {
      const votesData = await governanceService.getBudgetVotes(decodedName)
      votes.value = Array.isArray(votesData)
        ? [...votesData].sort((a, b) => (b.nTime || 0) - (a.nTime || 0))
        : []
    } catch (err) {
      votes.value = []
    }
  } catch (err) {
    error.value = 'Failed to load the proposal.'
  } finally {
    loading.value = false
  }
}

watch(
  () => route.params.name,
  (name) => {
    if (name) {
      fetchProposal(String(name))
    }
  },
  { immediate: true }
)
</script>

<style scoped>
.proposal-detail-page {
  animation: fadeIn 0.3s ease;
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

.back-link {
  display: inline-block;
  color: var(--text-accent);
  text-decoration: none;
  font-size: var(--text-sm);
  font-weight: var(--weight-bold);
  margin-bottom: var(--space-4);
}

.back-link:hover {
  color: var(--pivx-accent-dark);
}

.header-title {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
  flex-wrap: wrap;
}

.header-title h1 {
  font-size: var(--text-3xl);
  font-weight: var(--weight-extrabold);
  color: var(--text-primary);
  margin: 0;
  word-break: break-word;
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

.mono {
  font-family: var(--font-mono);
}

.two-column-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(360px, 1fr));
  gap: var(--space-6);
}

.payment-details {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.payment-item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--space-3);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
}

.payment-label {
  color: var(--text-secondary);
  font-size: var(--text-sm);
}

.payment-value {
  font-family: var(--font-mono);
  font-weight: var(--weight-bold);
  color: var(--text-accent);
}

.voting-stats {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.vote-bar {
  height: 12px;
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

.vote-summary {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.vote-item {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-3);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
}

.vote-label {
  font-size: var(--text-xs);
  color: var(--text-secondary);
  text-transform: uppercase;
  font-weight: var(--weight-bold);
  flex: 1;
}

.vote-value {
  font-size: var(--text-xl);
  font-weight: var(--weight-extrabold);
  color: var(--text-primary);
}

.vote-value.yes {
  color: var(--success);
}

.vote-value.no {
  color: var(--danger);
}

.vote-percentage {
  font-size: var(--text-sm);
  color: var(--text-tertiary);
}

.net-votes {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--space-4);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  border: 2px solid var(--border-secondary);
}

.net-label {
  font-weight: var(--weight-bold);
  color: var(--text-secondary);
}

.net-value {
  font-size: var(--text-2xl);
  font-weight: var(--weight-extrabold);
  font-family: var(--font-mono);
  color: var(--text-primary);
}

.net-value.positive {
  color: var(--success);
}

.net-value.negative {
  color: var(--danger);
}

.threshold-info {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--space-3);
  flex-wrap: wrap;
  padding: var(--space-4);
  background: var(--bg-elevated);
  border-radius: var(--radius-sm);
  border: 2px solid var(--border-secondary);
}

.threshold-item {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.threshold-label {
  font-size: var(--text-sm);
  color: var(--text-secondary);
}

.threshold-value {
  font-weight: var(--weight-bold);
  color: var(--text-accent);
  font-family: var(--font-mono);
}

.votes-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
}

.votes-header h2 {
  margin: 0;
}

.votes-list {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
  gap: var(--space-3);
  max-height: 600px;
  overflow-y: auto;
}

.vote-entry {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  padding: var(--space-3);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
}

.vote-mn {
  font-size: var(--text-xs);
  color: var(--text-primary);
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.vote-time {
  font-size: var(--text-xs);
  color: var(--text-tertiary);
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
  .detail-row {
    flex-direction: column;
  }

  .detail-value {
    text-align: left;
  }

  .two-column-grid {
    grid-template-columns: 1fr;
  }

  .votes-list {
    grid-template-columns: 1fr;
  }
}
</style>
