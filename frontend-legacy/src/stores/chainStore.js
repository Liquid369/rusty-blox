import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import api from '@/services/api'

export const useChainStore = defineStore('chain', () => {
  // State
  const height = ref(0)
  const syncHeight = ref(0)
  const networkHeight = ref(0)
  const networkHash = ref('')
  const syncPercentage = ref(0)
  const synced = ref(false)
  const loading = ref(false)
  const error = ref(null)
  const lastUpdate = ref(null)
  const reorgDetected = ref(false)
  const lastReorgInfo = ref(null)

  // Computed
  const blocksBehind = computed(() => {
    return Math.max(0, networkHeight.value - syncHeight.value)
  })

  const isSyncing = computed(() => {
    return syncPercentage.value < 100
  })

  // Actions
  async function fetchChainState() {
    loading.value = true
    error.value = null
    
    try {
      const response = await api.get('/api/v2/status')
      const data = response.data
      
      const newHeight = data.height || 0
      const newNetworkHeight = data.network_height || data.height || 0
      const newHash = data.hash || ''
      
      // Detect reorg: networkHeight decreased OR hash changed at same height
      const heightDecreased = networkHeight.value > 0 && newNetworkHeight < networkHeight.value
      const hashChanged = networkHeight.value > 0 && 
                         newNetworkHeight === networkHeight.value && 
                         networkHash.value && 
                         newHash && 
                         networkHash.value !== newHash
      
      if (heightDecreased || hashChanged) {
        console.warn('⚠️ REORG DETECTED', {
          oldHeight: networkHeight.value,
          newHeight: newNetworkHeight,
          oldHash: networkHash.value,
          newHash: newHash,
          type: heightDecreased ? 'height_decrease' : 'hash_change'
        })
        
        reorgDetected.value = true
        lastReorgInfo.value = {
          timestamp: Date.now(),
          oldHeight: networkHeight.value,
          newHeight: newNetworkHeight,
          oldHash: networkHash.value,
          newHash: newHash,
          type: heightDecreased ? 'height_decrease' : 'hash_change'
        }
        
        // Auto-clear reorg flag after 10 seconds
        setTimeout(() => {
          reorgDetected.value = false
        }, 10000)
      }
      
      height.value = newHeight
      syncHeight.value = newHeight
      networkHeight.value = newNetworkHeight
      networkHash.value = newHash
      syncPercentage.value = parseFloat(data.sync_percentage) || 0
      synced.value = data.synced || false
      lastUpdate.value = new Date()
      
      loading.value = false
    } catch (err) {
      error.value = err.message || 'Failed to fetch chain state'
      loading.value = false
      console.error('Chain state fetch error:', err)
    }
  }

  function $reset() {
    height.value = 0
    syncHeight.value = 0
    networkHeight.value = 0
    networkHash.value = ''
    syncPercentage.value = 0
    synced.value = false
    loading.value = false
    error.value = null
    lastUpdate.value = null
    reorgDetected.value = false
    lastReorgInfo.value = null
  }

  function clearReorgFlag() {
    reorgDetected.value = false
  }

  return {
    // State
    height,
    syncHeight,
    networkHeight,
    networkHash,
    syncPercentage,
    synced,
    loading,
    error,
    lastUpdate,
    reorgDetected,
    lastReorgInfo,
    // Computed
    blocksBehind,
    isSyncing,
    // Actions
    fetchChainState,
    clearReorgFlag,
    $reset
  }
})
