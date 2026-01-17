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
          icon="⚠️"
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
            <InfoRow label="Proposal Hash" icon="🔗">
              <HashDisplay :hash="proposal.Hash" :truncate="false" show-copy />
            </InfoRow>

            <InfoRow label="Fee Hash" icon="💳">
              <HashDisplay :hash="proposal.FeeHash" :truncate="false" show-copy />
            </InfoRow>

            <InfoRow label="Payment Address" icon="💰">
              <HashDisplay
                :hash="proposal.PaymentAddress"
                show-copy
                :link-to="`/address/${proposal.PaymentAddress}`"
              />
            </InfoRow>

            <InfoRow label="Forum URL" icon="🌐">
              <a :href="proposal.URL" target="_blank" class="external-link">
                {{ proposal.URL }} →
              </a>
            </InfoRow>

            <InfoRow label="Start Block" icon="🚀">
              {{ formatNumber(proposal.BlockStart) }}
            </InfoRow>

            <InfoRow label="End Block" icon="🏁">
              {{ formatNumber(proposal.BlockEnd) }}
            </InfoRow>

            <InfoRow label="Total Payment Count" icon="📝">
              {{ proposal.TotalPaymentCount }}
            </InfoRow>

            <InfoRow label="Remaining Payments" icon="⏳">
              {{ proposal.RemainingPaymentCount }}
            </InfoRow>

            <InfoRow label="Established" icon="✓">
              <Badge :variant="proposal.IsEstablished ? 'success' : 'warning'">
                {{ proposal.IsEstablished ? 'Yes' : 'No' }}
              </Badge>
            </InfoRow>

            <InfoRow label="Valid" icon="✅">
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
                  <span class="payment-value">{{ formatPIV(proposal.MonthlyPayment) }} PIV</span>
                  <span v-if="preferredCurrency !== 'PIV' && hasValidPrices" class="payment-fiat">
                    ≈ {{ formatAmount(proposal.MonthlyPayment, { showPIV: false }) }}
                  </span>
                </div>
              </div>
              <div class="payment-item">
                <span class="payment-label">Total Payment</span>
                <div class="payment-value-container">
                  <span class="payment-value">{{ formatPIV(proposal.TotalPayment) }} PIV</span>
                  <span v-if="preferredCurrency !== 'PIV' && hasValidPrices" class="payment-fiat">
                    ≈ {{ formatAmount(proposal.TotalPayment, { showPIV: false }) }}
                  </span>
                </div>
              </div>
              <div class="payment-item">
                <span class="payment-label">Allotted</span>
                <div class="payment-value-container">
                  <span class="payment-value">{{ formatPIV(proposal.Allotted) }} PIV</span>
                  <span v-if="preferredCurrency !== 'PIV' && hasValidPrices" class="payment-fiat">
                    ≈ {{ formatAmount(proposal.Allotted, { showPIV: false }) }}
                  </span>
                </div>
              </div>
              <div class="payment-item">
                <span class="payment-label">Ratio</span>
                <span class="payment-value">{{ proposal.Ratio.toFixed(2) }}</span>
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
                  <span class="vote-icon">👍</span>
                  <div class="vote-details">
                    <span class="vote-label">Yes Votes</span>
                    <span class="vote-value">{{ formatNumber(proposal.Yeas) }}</span>
                    <span class="vote-percentage">{{ getYeasPercentage(proposal).toFixed(1) }}%</span>
                  </div>
                </div>
                <div class="vote-item nays">
                  <span class="vote-icon">👎</span>
                  <div class="vote-details">
                    <span class="vote-label">No Votes</span>
                    <span class="vote-value">{{ formatNumber(proposal.Nays) }}</span>
                    <span class="vote-percentage">{{ getNaysPercentage(proposal).toFixed(1) }}%</span>
                  </div>
                </div>
                <div class="vote-item abstains">
                  <span class="vote-icon">🤷</span>
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
                  <span class="threshold-label">Required (10%):</span>
                  <span class="threshold-value">{{ formatNumber(passingThreshold) }}</span>
                </div>
                <div class="threshold-item">
                  <span class="threshold-label">Status:</span>
                  <Badge :variant="isProposalPassing ? 'success' : 'warning'">
                    {{ isProposalPassing ? 'Meeting Threshold' : 'Below Threshold' }}
                  </Badge>
                </div>
              </div>
            </div>
          </Card>
        </div>

        <!-- Individual Votes Section -->
        <Card v-if="votes" class="votes-card">
          <template #header>
            <div class="votes-header">
              <span>Individual Votes</span>
              <Badge variant="info">{{ Object.keys(votes).length }} masternodes</Badge>
            </div>
          </template>

          <div class="votes-list">
            <div v-for="(vote, address) in votes" :key="address" class="vote-entry">
              <HashDisplay
                :hash="address"
                :truncate="true"
                :start-length="10"
                :end-length="10"
                show-copy
              />
              <Badge :variant="getVoteVariant(vote)">{{ vote }}</Badge>
            </div>
          </div>
        </Card>
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, computed, onMounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useChainStore } from '@/stores/chainStore'
import { useCurrency } from '@/composables/useCurrency'
import { governanceService } from '@/services/governanceService'
import { masternodeService } from '@/services/masternodeService'
import { formatNumber, formatPIV } from '@/utils/formatters'
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

watch(() => route.params.hash, (newHash) => {
  if (newHash) {
    fetchProposal(newHash)
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
  background: var(--bg-tertiary);
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
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  border: 2px solid var(--border-secondary);
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
  font-weight: 700;
  color: var(--text-accent);
  font-family: var(--font-mono);
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
  background: var(--bg-tertiary);
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
