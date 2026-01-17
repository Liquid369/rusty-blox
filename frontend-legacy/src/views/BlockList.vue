<template>
  <AppLayout>
    <div class="block-list-page">
      <div class="page-header">
        <h1>Blocks</h1>
        <p class="page-subtitle">Browse all blocks in the PIVX blockchain</p>
      </div>

      <!-- Loading State -->
      <div v-if="loading && blocks.length === 0" class="loading-container">
        <LoadingSpinner size="lg" text="Loading blocks..." />
      </div>

      <!-- Error State -->
      <div v-else-if="error" class="error-container">
        <EmptyState
          icon="⚠️"
          title="Failed to Load Blocks"
          :message="error"
        >
          <template #action>
            <Button @click="fetchBlocks">Try Again</Button>
          </template>
        </EmptyState>
      </div>

      <!-- Blocks Grid -->
      <div v-else>
        <div class="blocks-grid">
          <BlockCard
            v-for="block in blocks"
            :key="block.height"
            :block="block"
            @click="navigateToBlock(block)"
          />
        </div>

        <!-- Pagination -->
        <Pagination
          v-if="blocks.length > 0"
          :current-page="currentPage"
          :page-size="pageSize"
          :total="totalBlocks"
          @update:page="changePage"
          @update:page-size="changePageSize"
        />
      </div>
    </div>
  </AppLayout>
</template>

<script setup>
import { ref, onMounted, watch } from 'vue'
import { useRouter } from 'vue-router'
import { useChainStore } from '@/stores/chainStore'
import { blockService } from '@/services/blockService'
import AppLayout from '@/components/layout/AppLayout.vue'
import BlockCard from '@/components/common/BlockCard.vue'
import LoadingSpinner from '@/components/common/LoadingSpinner.vue'
import EmptyState from '@/components/common/EmptyState.vue'
import Button from '@/components/common/Button.vue'
import Pagination from '@/components/common/Pagination.vue'

const router = useRouter()
const chainStore = useChainStore()

const blocks = ref([])
const loading = ref(false)
const error = ref('')
const currentPage = ref(1)
const pageSize = ref(25)
const totalBlocks = ref(0)

const fetchBlocks = async () => {
  loading.value = true
  error.value = ''
  
  try {
    await chainStore.fetchChainState()
    totalBlocks.value = chainStore.syncHeight
    
    // Calculate the range of blocks to fetch
    const endHeight = chainStore.syncHeight - (currentPage.value - 1) * pageSize.value
    const startHeight = Math.max(0, endHeight - pageSize.value + 1)
    
    // Fetch blocks in reverse order (newest first)
    const blockPromises = []
    for (let height = endHeight; height >= startHeight; height--) {
      blockPromises.push(blockService.getBlock(height))
    }
    
    const fetchedBlocks = await Promise.allSettled(blockPromises)
    blocks.value = fetchedBlocks
      .filter(result => result.status === 'fulfilled')
      .map(result => ({
        ...result.value,
        txCount: result.value.tx?.length || 0
      }))
  } catch (err) {
    console.error('Failed to fetch blocks:', err)
    error.value = err.message || 'Failed to load blocks'
  } finally {
    loading.value = false
  }
}

const changePage = (page) => {
  currentPage.value = page
  window.scrollTo({ top: 0, behavior: 'smooth' })
}

const changePageSize = (size) => {
  pageSize.value = size
  currentPage.value = 1
}

const navigateToBlock = (block) => {
  router.push(`/block/${block.height}`)
}

watch([currentPage, pageSize], () => {
  fetchBlocks()
})

onMounted(() => {
  fetchBlocks()
})
</script>

<style scoped>
.block-list-page {
  padding: var(--space-6);
}

.page-header {
  margin-bottom: var(--space-8);
}

.page-header h1 {
  margin-bottom: var(--space-2);
}

.page-subtitle {
  color: var(--text-secondary);
  font-size: var(--text-lg);
}

.blocks-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
  gap: var(--space-4);
  margin-bottom: var(--space-6);
}

.loading-container,
.error-container {
  min-height: 400px;
  display: flex;
  align-items: center;
  justify-content: center;
}

@media (max-width: 768px) {
  .block-list-page {
    padding: var(--space-4);
  }

  .blocks-grid {
    grid-template-columns: 1fr;
  }
}
</style>
