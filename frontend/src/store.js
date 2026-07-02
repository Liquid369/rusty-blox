/* =====================================================================
   PINIA — chain-status store. Mock by default (via api/client.js).
   The top bar + Dashboard read tip height / sync state from here.
   ===================================================================== */
import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { getStatus, getHealth } from './api/client.js'

export const useChainStore = defineStore('chain', () => {
  const height = ref(0)
  const networkHeight = ref(0)
  const hash = ref('')
  const synced = ref(false)
  const syncPercentage = ref(0)
  const health = ref(null)
  const loading = ref(false)
  const error = ref(null)
  const lastUpdate = ref(null)

  const blocksBehind = computed(() => Math.max(0, networkHeight.value - height.value))
  const isSyncing = computed(() => syncPercentage.value < 100)

  async function refresh() {
    loading.value = true
    error.value = null
    try {
      const s = await getStatus()
      height.value = s.height || 0
      networkHeight.value = s.network_height ?? s.height ?? 0
      hash.value = s.hash || ''
      synced.value = !!s.synced
      syncPercentage.value = Number(s.sync_percentage) || 0
      lastUpdate.value = new Date()
    } catch (e) {
      error.value = e.message || 'status fetch failed'
    } finally {
      loading.value = false
    }
  }

  async function refreshHealth() {
    try { health.value = await getHealth() } catch (e) { /* non-fatal */ }
  }

  return {
    height, networkHeight, hash, synced, syncPercentage, health,
    loading, error, lastUpdate, blocksBehind, isSyncing,
    refresh, refreshHealth
  }
})
