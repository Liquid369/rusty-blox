<template>
  <AppLayout>
    <div class="block-list-page">
      <h1>Latest Blocks</h1>

      <div v-if="loading" class="skeleton-list">
        <div v-for="i in 25" :key="i" class="skeleton" style="height: 100px; margin-bottom: var(--space-4);"></div>
      </div>

      <div v-else>
        <div class="blocks">
          <UiCard v-for="block in blocks" :key="block.height" hover clickable @click="goToBlock(block.height)">
            <div class="block-row">
              <div class="block-main">
                <span class="block-height">#{{ block.height.toLocaleString() }}</span>
                <span class="mono block-hash">{{ block.hash }}</span>
              </div>
              <div class="block-info">
                <span class="block-time">{{ formatDate(block.time) }}</span>
                <span class="block-txs">{{ block.tx_count }} transactions</span>
                <span class="block-size">{{ formatSize(block.size) }}</span>
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
import { blockService } from '@/services'
import AppLayout from '@/components/layout/AppLayout.vue'
import UiCard from '@/components/common/UiCard.vue'

const router = useRouter()

const loading = ref(true)
const blocks = ref([])
const itemsPerPage = 25

const formatDate = (timestamp) => {
  return new Date(timestamp * 1000).toLocaleString()
}

const formatSize = (bytes) => {
  return `${(bytes / 1024).toFixed(2)} KB`
}

const goToBlock = (height) => {
  router.push(`/block/${height}`)
}

const loadBlocks = async () => {
  loading.value = true
  try {
    const data = await blockService.getRecentBlocks(itemsPerPage)
    // API returns array directly
    blocks.value = Array.isArray(data) ? data : []
  } catch (error) {
    console.error('Failed to load blocks:', error)
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  loadBlocks()
})
</script>

<style scoped>
.block-list-page {
  animation: fadeIn 0.3s ease;
}

.blocks {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  margin-bottom: var(--space-8);
}

.block-row {
  display: flex;
  justify-content: space-between;
  gap: var(--space-6);
}

.block-main {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  flex: 1;
  min-width: 0;
}

.block-height {
  font-size: var(--text-xl);
  font-weight: var(--weight-bold);
  color: var(--text-accent);
}

.block-hash {
  color: var(--text-secondary);
  font-size: var(--text-sm);
  overflow: hidden;
  text-overflow: ellipsis;
}

.block-info {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: var(--space-1);
  font-size: var(--text-sm);
  color: var(--text-tertiary);
}

.block-time {
  color: var(--text-secondary);
}

@media (max-width: 768px) {
  .block-row {
    flex-direction: column;
  }

  .block-info {
    align-items: flex-start;
  }
}
</style>
