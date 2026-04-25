<template>
  <div class="pagination">
    <div class="pagination-info">
      Showing {{ startItem }}-{{ endItem }} of {{ formatNumber(total) }}
    </div>
    
    <div class="pagination-controls">
      <Button
        variant="ghost"
        size="sm"
        :disabled="currentPage === 1"
        @click="goToPage(1)"
      >
        ««
      </Button>
      
      <Button
        variant="ghost"
        size="sm"
        :disabled="currentPage === 1"
        @click="goToPage(currentPage - 1)"
      >
        ‹
      </Button>
      
      <div class="pagination-pages">
        <Button
          v-for="page in visiblePages"
          :key="page"
          :variant="page === currentPage ? 'primary' : 'ghost'"
          size="sm"
          @click="goToPage(page)"
        >
          {{ page }}
        </Button>
      </div>
      
      <Button
        variant="ghost"
        size="sm"
        :disabled="currentPage === totalPages"
        @click="goToPage(currentPage + 1)"
      >
        ›
      </Button>
      
      <Button
        variant="ghost"
        size="sm"
        :disabled="currentPage === totalPages"
        @click="goToPage(totalPages)"
      >
        »»
      </Button>
    </div>
    
    <div class="pagination-size">
      <label for="page-size">Per page:</label>
      <select 
        id="page-size"
        :value="pageSize"
        @change="updatePageSize($event.target.value)"
        class="page-size-select"
      >
        <option :value="25">25</option>
        <option :value="50">50</option>
        <option :value="100">100</option>
      </select>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import { formatNumber } from '@/utils/formatters'
import Button from './Button.vue'

const emit = defineEmits(['update:page', 'update:pageSize'])

const props = defineProps({
  currentPage: {
    type: Number,
    required: true
  },
  pageSize: {
    type: Number,
    required: true
  },
  total: {
    type: Number,
    required: true
  }
})

const totalPages = computed(() => Math.ceil(props.total / props.pageSize))

const startItem = computed(() => {
  return (props.currentPage - 1) * props.pageSize + 1
})

const endItem = computed(() => {
  return Math.min(props.currentPage * props.pageSize, props.total)
})

const visiblePages = computed(() => {
  const pages = []
  const maxVisible = 5
  
  let start = Math.max(1, props.currentPage - Math.floor(maxVisible / 2))
  let end = Math.min(totalPages.value, start + maxVisible - 1)
  
  if (end - start + 1 < maxVisible) {
    start = Math.max(1, end - maxVisible + 1)
  }
  
  for (let i = start; i <= end; i++) {
    pages.push(i)
  }
  
  return pages
})

const goToPage = (page) => {
  if (page >= 1 && page <= totalPages.value && page !== props.currentPage) {
    emit('update:page', page)
  }
}

const updatePageSize = (size) => {
  emit('update:pageSize', parseInt(size))
}
</script>

<style scoped>
.pagination {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
  padding: var(--space-4);
  background: var(--bg-secondary);
  border: 2px solid var(--border-secondary);
  border-radius: var(--radius-lg);
  flex-wrap: wrap;
}

.pagination-info {
  font-size: var(--text-sm);
  color: var(--text-secondary);
  font-weight: var(--weight-medium);
}

.pagination-controls {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.pagination-pages {
  display: flex;
  gap: var(--space-1);
}

.pagination-size {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--text-sm);
  color: var(--text-secondary);
}

.page-size-select {
  background: var(--bg-tertiary);
  color: var(--text-primary);
  border: 1px solid var(--border-primary);
  border-radius: var(--radius-sm);
  padding: var(--space-2) var(--space-3);
  font-size: var(--text-sm);
  cursor: pointer;
  transition: border-color var(--transition-fast);
}

.page-size-select:hover {
  border-color: var(--border-accent);
}

.page-size-select:focus {
  outline: 2px solid var(--border-accent);
  outline-offset: 2px;
}

@media (max-width: 768px) {
  .pagination {
    flex-direction: column;
    gap: var(--space-3);
  }

  .pagination-info,
  .pagination-size {
    width: 100%;
    justify-content: center;
  }

  .pagination-controls {
    width: 100%;
    justify-content: center;
  }
}
</style>
