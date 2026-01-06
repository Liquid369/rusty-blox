import { defineStore } from 'pinia'
import { ref, computed } from 'vue'

export const useChainStore = defineStore('chain', () => {
  const height = ref(0)
  const syncPercentage = ref(100)
  const supply = ref('0')
  const masternodeCount = ref(0)
  const lastUpdate = ref(null)

  const isSynced = computed(() => syncPercentage.value >= 100)

  const updateChainInfo = (data) => {
    if (data.height) height.value = data.height
    if (data.syncPercentage !== undefined) syncPercentage.value = data.syncPercentage
    if (data.supply) supply.value = data.supply
    if (data.masternodeCount !== undefined) masternodeCount.value = data.masternodeCount
    lastUpdate.value = Date.now()
  }

  return {
    height,
    syncPercentage,
    supply,
    masternodeCount,
    lastUpdate,
    isSynced,
    updateChainInfo
  }
})
