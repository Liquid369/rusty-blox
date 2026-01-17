<template>
  <AppLayout>
    <div class="dashboard">
      <h1>PIVX Blockchain Explorer</h1>

      <!-- Network Stats -->
      <div class="stats-grid">
        <StatCard 
          label="Block Height" 
          :value="chainStore.height" 
          format="number"
          :loading="loading"
        />
        <StatCard 
          label="Sync Status" 
          :value="chainStore.isSynced ? '✓ Synced' : `${chainStore.syncPercentage.toFixed(1)}%`"
          :loading="loading"
        />
        <StatCard 
          label="Total Supply" 
          :value="formatPIV(chainStore.supply)"
          subtitle="PIV"
          :loading="loading"
        />
        <StatCard 
          label="Masternodes" 
          :value="chainStore.masternodeCount" 
          format="number"
          :loading="loading"
        />
      </div>

      <!-- Recent Blocks -->
      <div class="activity-section">
        <div class="section-header">
          <h2>Recent Blocks</h2>
          <router-link to="/blocks" class="view-all-link">View All →</router-link>
        </div>
        
        <div v-if="loadingBlocks" class="skeleton-list">
          <div v-for="i in 5" :key="i" class="skeleton" style="height: 80px; margin-bottom: 16px;"></div>
        </div>
        
        <div v-else class="block-list">
          <UiCard v-for="block in recentBlocks" :key="block.height" hover clickable @click="goToBlock(block.height)">
            <div class="block-card-content">
              <div class="block-header">
                <span class="block-height">#{{ block.height.toLocaleString() }}</span>
                <span class="block-time">{{ formatTimeAgo(block.time) }}</span>
              </div>
              <div class="block-meta">
                <span class="mono block-hash">{{ truncateHash(block.hash) }}</span>
                <span class="block-txs">{{ block.tx_count }} txs</span>
              </div>
            </div>
          </UiCard>
        </div>
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { useChainStore } from '@/stores/chainStore'
import { blockService, transactionService, chainService } from '@/services'
import AppLayout from '@/components/layout/AppLayout.vue'
import StatCard from '@/components/common/StatCard.vue'
import UiCard from '@/components/common/UiCard.vue'

const router = useRouter()
const chainStore = useChainStore()

const loading = ref(false)
const loadingBlocks = ref(false)
const loadingTxs = ref(false)
const recentBlocks = ref([])
const recentTxs = ref([])

console.log('Dashboard component mounted')
console.log('Initial chainStore values:', {
  height: chainStore.height,
  syncPercentage: chainStore.syncPercentage,
  supply: chainStore.supply,
  masternodeCount: chainStore.masternodeCount
})
console.log('Initial recentBlocks:', recentBlocks.value)

const formatPIV = (sats) => {
  return (Number(sats) / 100000000).toFixed(2)
}

const formatTimeAgo = (timestamp) => {
  const now = Date.now()
  const diff = now - (timestamp * 1000)
  const minutes = Math.floor(diff / 60000)
  
  if (minutes < 1) return 'just now'
  if (minutes === 1) return '1 min ago'
  if (minutes < 60) return `${minutes} mins ago`
  
  const hours = Math.floor(minutes / 60)
  if (hours === 1) return '1 hour ago'
  if (hours < 24) return `${hours} hours ago`
  
  const days = Math.floor(hours / 24)
  return `${days} day${days > 1 ? 's' : ''} ago`
}

const truncateHash = (hash) => {
  if (!hash) return ''
  return `${hash.slice(0, 8)}...${hash.slice(-8)}`
}

const goToBlock = (height) => {
  router.push(`/block/${height}`)
}

const goToTx = (txid) => {
  router.push(`/tx/${txid}`)
}

const loadData = async () => {
  console.log('loadData() called')
  try {
    // Load chain info
    console.log('Fetching status...')
    const chainInfo = await chainService.getStatus()
    console.log('Chain info received:', chainInfo)
    
    if (chainInfo && chainInfo.height) {
      const updateData = {
        height: chainInfo.height || 0,
        syncPercentage: chainInfo.synced ? 100 : (chainInfo.sync_percentage || 0),
        supply: '0', // TODO: Get supply from API
        masternodeCount: 0 // TODO: Get masternode count from API
      }
      console.log('Updating chainStore with:', updateData)
      chainStore.updateChainInfo(updateData)
      console.log('ChainStore after update:', {
        height: chainStore.height,
        syncPercentage: chainStore.syncPercentage
      })
    }
    loading.value = false

    // Load recent blocks - API returns array directly
    console.log('Fetching blocks...')
    const blocksData = await blockService.getRecentBlocks(5)
    console.log('Blocks data received:', blocksData)
    console.log('Is array?', Array.isArray(blocksData))
    // blocksData is already an array of blocks
    recentBlocks.value = Array.isArray(blocksData) ? blocksData : []
    console.log('recentBlocks.value set to:', recentBlocks.value)
    console.log('recentBlocks.value.length:', recentBlocks.value.length)
    loadingBlocks.value = false

    // Load recent transactions - for now use blocks' tx data
    // TODO: Implement proper recent transactions endpoint
    recentTxs.value = []
    loadingTxs.value = false
  } catch (error) {
    console.error('Failed to load dashboard data:', error)
    // Set loading to false so content can render even on error
    loading.value = false
    loadingBlocks.value = false
    loadingTxs.value = false
    
    // Set default values so UI doesn't stay blank
    if (chainStore.height === 0) {
      chainStore.updateChainInfo({
        height: 0,
        syncPercentage: 0,
        supply: '0',
        masternodeCount: 0
      })
    }
  }
}

onMounted(() => {
  loadData()
})
</script>

<style scoped>
.dashboard {
  animation: fadeIn 0.3s ease;
}

.dashboard h1 {
  font-size: var(--text-4xl);
  font-weight: var(--weight-extrabold);
  margin-bottom: var(--space-8);
  color: var(--text-primary);
}

.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
  gap: var(--space-6);
  margin-bottom: var(--space-12);
}

.activity-section {
  margin-bottom: var(--space-8);
}

.section-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--space-6);
  padding-bottom: var(--space-4);
  border-bottom: 2px solid var(--border-primary);
}

.section-header h2 {
  font-size: var(--text-2xl);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
  margin: 0;
}

.view-all-link {
  font-size: var(--text-sm);
  font-weight: var(--weight-bold);
  color: var(--text-accent);
  text-decoration: none;
  transition: color var(--transition-fast);
}

.view-all-link:hover {
  color: var(--pivx-accent-dark);
}

.block-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.block-card-content {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.block-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--space-2);
}

.block-height {
  font-size: var(--text-xl);
  font-weight: var(--weight-extrabold);
  color: var(--text-accent);
  letter-spacing: -0.02em;
}

.block-time {
  font-size: var(--text-sm);
  color: var(--text-tertiary);
  font-weight: var(--weight-medium);
}

.block-meta {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--space-4);
}

.mono {
  font-family: var(--font-mono);
}

.block-hash {
  font-size: var(--text-sm);
  color: var(--text-secondary);
  flex: 1;
  min-width: 0;
}

.block-txs {
  font-size: var(--text-sm);
  color: var(--text-tertiary);
  font-weight: var(--weight-semibold);
  white-space: nowrap;
  padding: var(--space-1) var(--space-3);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
}

.skeleton-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.skeleton {
  background: linear-gradient(
    90deg,
    var(--bg-secondary) 25%,
    var(--bg-tertiary) 50%,
    var(--bg-secondary) 75%
  );
  background-size: 200% 100%;
  animation: loading 1.5s ease-in-out infinite;
  border-radius: var(--radius-md);
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

@keyframes loading {
  0% {
    background-position: 200% 0;
  }
  100% {
    background-position: -200% 0;
  }
}

.block-time {
  font-size: var(--text-sm);
  color: var(--text-tertiary);
}

.block-meta {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: var(--text-sm);
}

.block-hash {
  color: var(--text-secondary);
}

.block-txs {
  color: var(--text-tertiary);
}

.tx-card-content {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.tx-id {
  color: var(--text-accent);
  font-size: var(--text-sm);
}

.tx-time {
  font-size: var(--text-sm);
  color: var(--text-tertiary);
}

.skeleton-list {
  display: flex;
  flex-direction: column;
}

@media (max-width: 768px) {
  .stats-grid {
    grid-template-columns: 1fr;
  }

  .activity-grid {
    grid-template-columns: 1fr;
  }
}
</style>
