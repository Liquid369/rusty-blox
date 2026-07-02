/* =====================================================================
   PINIA — chain-status store. Mock by default (via api/client.js).
   The top bar + Dashboard read tip height / sync state from here.
   ===================================================================== */
import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { getStatus, getHealth, API_BASE, isMock } from './api/client.js'

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
  const lastBlockAt = ref(0) // unix secs of the most recent block, driven by the WS feed

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

  // --- live block feed over WebSocket (/ws/blocks) ----------------------
  // The instant the explorer connects a block it broadcasts NewBlock with a
  // wall-clock `timestamp`. Driving lastBlockAt from this avoids the ~60s
  // /block-stats cache lag that made "since last block" climb. Auto-reconnects.
  let ws = null
  let wsRetry = null

  function connectLive() {
    if (isMock || ws) return
    const httpBase = API_BASE || `${location.protocol}//${location.host}`
    const url = httpBase.replace(/^http/, 'ws') + '/ws/blocks'
    try {
      ws = new WebSocket(url)
    } catch {
      scheduleReconnect()
      return
    }
    ws.onmessage = (e) => {
      try {
        const ev = JSON.parse(e.data)
        if (ev.type === 'NewBlock') {
          if (ev.height > height.value) height.value = ev.height
          if (ev.height > networkHeight.value) networkHeight.value = ev.height
          // Use the client clock at receipt (matches `now` in the counter) so the
          // reading is immune to client<->server clock skew; the event arrives the
          // moment the explorer connects the block.
          lastBlockAt.value = Math.floor(Date.now() / 1000)
        }
      } catch { /* ignore malformed frames */ }
    }
    ws.onclose = () => { ws = null; scheduleReconnect() }
    ws.onerror = () => { try { ws && ws.close() } catch (_) { /* noop */ } }
  }

  function scheduleReconnect() {
    if (wsRetry) return
    wsRetry = setTimeout(() => { wsRetry = null; connectLive() }, 5000)
  }

  function disconnectLive() {
    if (wsRetry) { clearTimeout(wsRetry); wsRetry = null }
    if (ws) { try { ws.onclose = null; ws.close() } catch (_) { /* noop */ } ws = null }
  }

  return {
    height, networkHeight, hash, synced, syncPercentage, health,
    loading, error, lastUpdate, lastBlockAt, blocksBehind, isSyncing,
    refresh, refreshHealth, connectLive, disconnectLive
  }
})
