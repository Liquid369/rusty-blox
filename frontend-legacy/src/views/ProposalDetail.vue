<template>
  <AppLayout>
    <div class="proposal-detail-page">
      <!-- Loading State -->
      <div v-if="loading" class="loading-container">
        <LoadingSpinner size="lg" text="Loading proposal..." />
      </div>

      <!-- Error State -->
      <div v-else-if="error" class="error-container">
        <EmptyState
          icon="alert-triangle"
          title="Proposal Not Found"
          :message="error"
        >
          <template #action>
            <Button @click="$router.push('/governance')">View All Proposals</Button>
          </template>
        </EmptyState>
      </div>

      <!-- Proposal Details -->
      <div v-else-if="proposal">
        <!-- Header -->
        <div class="page-header">
          <Button
            variant="ghost"
            @click="$router.push('/governance')"
          >
            ← Back to Governance
          </Button>
          <div class="header-title">
            <h1>{{ proposal.Name }}</h1>
            <Badge :variant="getProposalVariant(proposal)">
              {{ getProposalStatus(proposal) }}
            </Badge>
          </div>
        </div>

        <!-- Main Info Card -->
        <Card class="main-info-card">
          <template #header>Proposal Information</template>

          <div class="info-grid">
            <InfoRow label="Proposal Hash" icon="link">
              <HashDisplay :hash="proposal.Hash" :truncate="false" show-copy />
            </InfoRow>

            <InfoRow label="Fee Hash" icon="credit-card">
              <HashDisplay :hash="proposal.FeeHash" :truncate="false" show-copy />
            </InfoRow>

            <InfoRow label="Payment Address" icon="coins">
              <HashDisplay
                :hash="proposal.PaymentAddress"
                show-copy
                :link-to="`/address/${proposal.PaymentAddress}`"
              />
            </InfoRow>

            <InfoRow label="Forum URL" icon="globe">
              <a
                v-if="safeUrl"
                :href="safeUrl"
                target="_blank"
                rel="noopener noreferrer"
                class="external-link"
              >
                {{ proposal.URL }} →
              </a>
              <span v-else class="external-link-disabled">{{ proposal.URL || '—' }}</span>
            </InfoRow>

            <InfoRow label="Start Block" icon="play">
              {{ formatNumber(proposal.BlockStart) }}
            </InfoRow>

            <InfoRow label="End Block" icon="flag">
              {{ formatNumber(proposal.BlockEnd) }}
            </InfoRow>

            <InfoRow label="Total Payment Count" icon="file-text">
              {{ proposal.TotalPaymentCount }}
            </InfoRow>

            <InfoRow label="Remaining Payments" icon="hourglass">
              {{ proposal.RemainingPaymentCount }}
              <span class="remaining-note">({{ proposal.RemainingPaymentCount }} month{{ proposal.RemainingPaymentCount !== 1 ? 's' : '' }} left)</span>
            </InfoRow>

            <InfoRow label="Established" icon="check">
              <Badge :variant="proposal.IsEstablished ? 'success' : 'warning'">
                {{ proposal.IsEstablished ? 'Yes' : 'No' }}
              </Badge>
            </InfoRow>

            <InfoRow label="Valid" icon="check-circle">
              <Badge :variant="proposal.IsValid ? 'success' : 'danger'">
                {{ proposal.IsValid ? 'Yes' : 'No' }}
              </Badge>
            </InfoRow>
          </div>
        </Card>

        <!-- Payment Info -->
        <div class="two-column-grid">
          <Card>
            <template #header>Payment Details</template>
            <div class="payment-details">
              <div class="payment-item">
                <span class="payment-label">Monthly Payment</span>
                <div class="payment-value-container">
                  <span class="payment-value">{{ formatNumber(proposal.MonthlyPayment) }} PIV</span>
                  <span v-if="preferredCurrency !== 'PIV' && hasValidPrices" class="payment-fiat">
                    ≈ {{ formatAmount(proposal.MonthlyPayment, { showPIV: false }) }}
                  </span>
                </div>
              </div>
              <div class="payment-item">
                <span class="payment-label">Total Payment</span>
                <div class="payment-value-container">
                  <span class="payment-value">{{ formatNumber(proposal.TotalPayment) }} PIV</span>
                  <span v-if="preferredCurrency !== 'PIV' && hasValidPrices" class="payment-fiat">
                    ≈ {{ formatAmount(proposal.TotalPayment, { showPIV: false }) }}
                  </span>
                </div>
              </div>
              <div class="payment-item">
                <span class="payment-label">Allotted</span>
                <div class="payment-value-container">
                  <span class="payment-value">{{ formatNumber(proposal.Allotted) }} PIV</span>
                  <span v-if="preferredCurrency !== 'PIV' && hasValidPrices" class="payment-fiat">
                    ≈ {{ formatAmount(proposal.Allotted, { showPIV: false }) }}
                  </span>
                </div>
              </div>
              <div class="payment-item">
                <span class="payment-label">Remaining Payout</span>
                <div class="payment-value-container">
                  <span class="payment-value">{{ formatNumber(remainingPayout) }} PIV</span>
                  <span class="payment-fiat">{{ proposal.RemainingPaymentCount }} × {{ formatNumber(proposal.MonthlyPayment) }} PIV</span>
                </div>
              </div>
              <div class="payment-item">
                <span class="payment-label">Yes Ratio</span>
                <span class="payment-value">{{ Number(proposal.Ratio || 0).toFixed(2) }}</span>
              </div>

              <!-- Funding utilization vs the monthly treasury cap -->
              <div class="utilization-block">
                <div class="utilization-header">
                  <span class="payment-label">Budget Utilization</span>
                  <span class="utilization-value">{{ budgetSharePercent }}% of {{ formatNumber(monthlyBudgetCap) }} PIV cap</span>
                </div>
                <div class="utilization-bar">
                  <div
                    class="utilization-bar-fill"
                    :style="{ width: Math.min(100, Number(budgetSharePercent)) + '%' }"
                  ></div>
                </div>
              </div>
            </div>
          </Card>

          <Card>
            <template #header>Voting Statistics</template>
            <div class="voting-stats">
              <!-- Vote Bar -->
              <div class="vote-bar-container">
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
              </div>

              <!-- Vote Numbers -->
              <div class="vote-numbers">
                <div class="vote-item yeas">
                  <span class="vote-icon"><Icon name="thumbs-up" :size="20" /></span>
                  <div class="vote-details">
                    <span class="vote-label">Yes Votes</span>
                    <span class="vote-value">{{ formatNumber(proposal.Yeas) }}</span>
                    <span class="vote-percentage">{{ getYeasPercentage(proposal).toFixed(1) }}%</span>
                  </div>
                </div>
                <div class="vote-item nays">
                  <span class="vote-icon"><Icon name="thumbs-down" :size="20" /></span>
                  <div class="vote-details">
                    <span class="vote-label">No Votes</span>
                    <span class="vote-value">{{ formatNumber(proposal.Nays) }}</span>
                    <span class="vote-percentage">{{ getNaysPercentage(proposal).toFixed(1) }}%</span>
                  </div>
                </div>
                <div class="vote-item abstains">
                  <span class="vote-icon"><Icon name="help-circle" :size="20" /></span>
                  <div class="vote-details">
                    <span class="vote-label">Abstain</span>
                    <span class="vote-value">{{ formatNumber(proposal.Abstains) }}</span>
                  </div>
                </div>
              </div>

              <!-- Net Votes -->
              <div class="net-votes">
                <span class="net-label">Net Votes:</span>
                <span :class="['net-value', { positive: netVotes > 0, negative: netVotes < 0 }]">
                  {{ netVotes > 0 ? '+' : '' }}{{ formatNumber(netVotes) }}
                </span>
              </div>

              <!-- Passing Threshold Info -->
              <div v-if="mnCount" class="threshold-info">
                <div class="threshold-item">
                  <span class="threshold-label">Required (10% of {{ formatNumber(mnCount.enabled) }} MNs):</span>
                  <span class="threshold-value">{{ formatNumber(passingThreshold) }}</span>
                </div>
                <div class="threshold-item">
                  <span class="threshold-label">Status:</span>
                  <Badge :variant="isProposalPassing ? 'success' : 'warning'">
                    {{ isProposalPassing ? 'Meeting Threshold' : 'Below Threshold' }}
                  </Badge>
                </div>
              </div>

              <!-- Margin to pass -->
              <div v-if="mnCount" class="threshold-info">
                <div class="threshold-item">
                  <span class="threshold-label">Margin to Pass:</span>
                  <span :class="['threshold-value', voteMargin >= 0 ? 'margin-positive' : 'margin-negative']">
                    {{ voteMargin >= 0 ? '+' : '' }}{{ formatNumber(voteMargin) }} votes
                  </span>
                </div>
                <div class="threshold-item">
                  <Badge :variant="voteMargin >= 0 ? 'success' : 'danger'" size="sm">
                    {{ voteMargin >= 0 ? `${formatNumber(voteMargin)} above threshold` : `needs ${formatNumber(-voteMargin)} more` }}
                  </Badge>
                </div>
              </div>
            </div>
          </Card>
        </div>

        <!-- Individual Votes Section -->
        <Card v-if="voteEntries.length > 0" class="votes-card">
          <template #header>
            <div class="votes-header">
              <span>Individual Votes</span>
              <Badge variant="info">{{ voteEntries.length }} masternodes</Badge>
            </div>
          </template>

          <div class="votes-list">
            <div v-for="entry in voteEntries" :key="entry.id" class="vote-entry">
              <HashDisplay
                :hash="entry.id"
                :truncate="true"
                :start-length="10"
                :end-length="10"
                show-copy
              />
              <Badge :variant="getVoteVariant(entry.vote)">{{ entry.vote }}</Badge>
            </div>
          </div>
        </Card>
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import Icon from '@/components/common/Icon.vue'
import { ref, computed, onMounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useChainStore } from '@/stores/chainStore'
import { useCurrency } from '@/composables/useCurrency'
import { governanceService } from '@/services/governanceService'
import { masternodeService } from '@/services/masternodeService'
import { formatNumber } from '@/utils/formatters'
import { PIVX_GOVERNANCE } from '@/utils/governanceStatus'
import AppLayout from '@/components/layout/AppLayout.vue'
import Card from '@/components/common/Card.vue'
import Badge from '@/components/common/Badge.vue'
import Button from '@/components/common/Button.vue'
import InfoRow from '@/components/common/InfoRow.vue'
import HashDisplay from '@/components/common/HashDisplay.vue'
import LoadingSpinner from '@/components/common/LoadingSpinner.vue'
import EmptyState from '@/components/common/EmptyState.vue'

const route = useRoute()
const router = useRouter()
const chainStore = useChainStore()
const { formatAmount, preferredCurrency, hasValidPrices } = useCurrency()

const proposal = ref(null)
const votes = ref(null)
const mnCount = ref(null)
const loading = ref(false)
const error = ref('')

// Check if proposal is completed (voting period has ended)
const isProposalCompleted = computed(() => {
  if (!proposal.value) return false
  // Only completed if proposal period has ended, not just payments
  return chainStore.syncHeight && chainStore.syncHeight >= proposal.value.BlockEnd
})

const netVotes = computed(() => {
  if (!proposal.value) return 0
  return proposal.value.Yeas - proposal.value.Nays
})

// Calculate 10% threshold for passing
const passingThreshold = computed(() => {
  if (!mnCount.value) return 0
  return Math.ceil(mnCount.value.enabled * 0.10)
})

const isProposalPassing = computed(() => {
  if (!proposal.value || !proposal.value.IsValid || isProposalCompleted.value) return false
  return netVotes.value >= passingThreshold.value
})

// How far above (or below) the 10% passing threshold this proposal sits
const voteMargin = computed(() => netVotes.value - passingThreshold.value)

const monthlyBudgetCap = PIVX_GOVERNANCE.MAX_MONTHLY_BUDGET

// Share of the monthly treasury cap this proposal consumes
const budgetSharePercent = computed(() => {
  if (!proposal.value) return '0.0'
  return (((proposal.value.MonthlyPayment || 0) / monthlyBudgetCap) * 100).toFixed(1)
})

// PIV still owed across the remaining payment months
const remainingPayout = computed(() => {
  if (!proposal.value) return 0
  return (proposal.value.RemainingPaymentCount || 0) * (proposal.value.MonthlyPayment || 0)
})

const getProposalStatus = (proposal) => {
  if (!proposal.IsValid) return 'Invalid'
  if (isProposalCompleted.value) return 'Completed'
  if (isProposalPassing.value) return 'Passing'
  return 'Failing'
}

const getProposalVariant = (proposal) => {
  if (!proposal.IsValid) return 'danger'
  if (isProposalCompleted.value) return 'secondary'
  if (isProposalPassing.value) return 'success'
  return 'warning'
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

const getVoteVariant = (vote) => {
  if (vote === 'YES') return 'success'
  if (vote === 'NO') return 'danger'
  return 'secondary'
}

// The proposal URL is attacker-controllable: only expose plain http(s) links,
// never javascript:/data: or other schemes.
const safeUrl = computed(() => {
  const url = proposal.value?.URL
  if (typeof url !== 'string') return ''
  const trimmed = url.trim()
  return /^https?:\/\//i.test(trimmed) ? trimmed : ''
})

// Normalize the votes payload into [{ id, vote }] regardless of whether the
// API returns an array of vote records or an { id: voteString } map.
const voteEntries = computed(() => {
  const raw = votes.value
  if (!raw) return []
  if (Array.isArray(raw)) {
    return raw.map((v, i) => ({
      id: v.mnId || v.nHash || String(i),
      vote: v.Vote || ''
    }))
  }
  return Object.entries(raw).map(([id, vote]) => ({ id, vote }))
})

const fetchProposal = async (proposalName) => {
  loading.value = true
  error.value = ''
  proposal.value = null
  votes.value = null
  mnCount.value = null

  try {
    const decodedName = decodeURIComponent(proposalName)
    
    // Fetch budget info, masternode count, chain state, and votes in parallel
    const [budgetInfo, mnCountData] = await Promise.all([
      governanceService.getBudgetInfo(),
      masternodeService.getMasternodeCount(),
      chainStore.fetchChainState()
    ])
    
    mnCount.value = mnCountData
    
    const found = budgetInfo.find(p => p.Name === decodedName)
    if (!found) {
      throw new Error('Proposal not found')
    }

    proposal.value = found

    // Fetch votes
    try {
      const votesData = await governanceService.getBudgetVotes(decodedName)
      votes.value = votesData
    } catch (err) {
      console.warn('Failed to fetch votes:', err)
      // Continue without votes
    }
  } catch (err) {
    console.error('Failed to fetch proposal:', err)
    error.value = err.message || 'Failed to load proposal'
  } finally {
    loading.value = false
  }
}

watch(() => route.params.name, (newName) => {
  if (newName) {
    fetchProposal(newName)
  }
}, { immediate: true })
</script>

<style scoped>
.proposal-detail-page {
  padding: var(--space-6);
  max-width: 1400px;
  margin: 0 auto;
}

.page-header {
  margin-bottom: var(--space-6);
}

.header-title {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
  margin-top: var(--space-4);
}

.header-title h1 {
  margin: 0;
  flex: 1;
}

.main-info-card {
  margin-bottom: var(--space-6);
}

.info-grid {
  display: grid;
  gap: var(--space-4);
}

.external-link {
  color: var(--text-accent);
  text-decoration: none;
  transition: opacity 0.2s;
  word-break: break-all;
}

.external-link:hover {
  opacity: 0.8;
}

.external-link-disabled {
  color: var(--text-tertiary);
  word-break: break-all;
}

.two-column-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(400px, 1fr));
  gap: var(--space-4);
  margin-bottom: var(--space-6);
}

.payment-details {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.payment-item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--space-3);
  background: rgba(var(--rgb-purple-dark), 0.5);
  border-radius: var(--radius-sm);
}

.payment-label {
  color: var(--text-secondary);
  font-size: var(--text-sm);
}

.payment-value-container {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.payment-value {
  font-family: var(--font-mono);
  font-weight: 700;
  color: var(--text-accent);
  font-size: var(--text-lg);
}

.payment-fiat {
  font-size: var(--text-sm);
  color: var(--text-secondary);
  font-weight: var(--weight-normal);
}

.voting-stats {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.vote-bar-container {
  padding: var(--space-2) 0;
}

.vote-bar {
  height: 12px;
  background: var(--bg-tertiary);
  border-radius: var(--radius-full);
  overflow: hidden;
  display: flex;
  border: 1px solid var(--border-secondary);
  box-shadow: inset 0 1px 2px rgba(var(--rgb-purple-darkest), 0.5);
}

.vote-bar-fill {
  height: 100%;
  transition: width var(--transition-slow);
}

.vote-bar-fill.yeas {
  background: linear-gradient(90deg, var(--green-accent-dark) 0%, var(--success) 100%);
}

.vote-bar-fill.nays {
  background: linear-gradient(90deg, var(--danger) 0%, #f87171 100%);
}

.vote-numbers {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.vote-item {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-3);
  background: rgba(var(--rgb-purple-dark), 0.5);
  border-radius: var(--radius-sm);
}

.vote-icon {
  font-size: var(--text-2xl);
}

.vote-details {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  flex: 1;
}

.vote-label {
  font-size: var(--text-xs);
  color: var(--text-secondary);
  text-transform: uppercase;
}

.vote-value {
  font-size: var(--text-xl);
  font-weight: 700;
  color: var(--text-primary);
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
  background: rgba(var(--rgb-purple-dark), 0.5);
  border-radius: var(--radius-sm);
  border: 1px solid var(--border-primary);
}

.net-label {
  font-weight: 600;
  color: var(--text-secondary);
}

.net-value {
  font-size: var(--text-2xl);
  font-weight: 700;
  font-family: var(--font-mono);
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
  padding: var(--space-4);
  background: var(--bg-elevated);
  border-radius: var(--radius-sm);
  border: 1px solid var(--border-primary);
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
  font-weight: 700;
  color: var(--text-accent);
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
}

.margin-positive {
  color: var(--success);
}

.margin-negative {
  color: var(--danger);
}

.remaining-note {
  font-size: var(--text-xs);
  color: var(--text-tertiary);
  margin-left: var(--space-1);
}

.utilization-block {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: var(--space-3);
  background: rgba(var(--rgb-purple-dark), 0.5);
  border-radius: var(--radius-sm);
}

.utilization-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--space-3);
  flex-wrap: wrap;
}

.utilization-value {
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
  font-size: var(--text-sm);
  font-weight: var(--weight-bold);
  color: var(--text-accent);
}

.utilization-bar {
  height: 8px;
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  overflow: hidden;
  border: 1px solid var(--border-secondary);
}

.utilization-bar-fill {
  height: 100%;
  background: linear-gradient(90deg, var(--pivx-purple-primary) 0%, var(--pivx-accent) 100%);
  transition: width 0.5s ease-out;
}

.votes-card {
  margin-top: var(--space-6);
}

.votes-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
}

.votes-list {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  gap: var(--space-3);
  max-height: 600px;
  overflow-y: auto;
}

.vote-entry {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-3);
  background: rgba(var(--rgb-purple-dark), 0.5);
  border-radius: var(--radius-sm);
}

.loading-container,
.error-container {
  min-height: 400px;
  display: flex;
  align-items: center;
  justify-content: center;
}

@media (max-width: 768px) {
  .proposal-detail-page {
    padding: var(--space-4);
  }

  .header-title {
    flex-direction: column;
    align-items: flex-start;
  }

  .two-column-grid {
    grid-template-columns: 1fr;
  }

  .votes-list {
    grid-template-columns: 1fr;
  }
}
</style>
