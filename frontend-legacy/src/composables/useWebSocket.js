import { ref, onUnmounted } from 'vue'

/**
 * WebSocket composable with auto-reconnect and event handling
 * 
 * @param {string} endpoint - WebSocket endpoint (e.g., '/ws/blocks')
 * @param {object} options - Configuration options
 * @returns {object} WebSocket connection utilities
 */
export function useWebSocket(endpoint, options = {}) {
  const {
    autoConnect = true,
    reconnect = true,
    reconnectInterval = 1000,
    maxReconnectInterval = 30000,
    reconnectDecay = 1.5,
    maxReconnectAttempts = 20,
    onOpen = null,
    onClose = null,
    onError = null,
    onMessage = null,
    onReconnectFailed = null
  } = options

  const ws = ref(null)
  const connected = ref(false)
  const connecting = ref(false)
  const reconnectAttempts = ref(0)
  const reconnectFailed = ref(false)
  const currentReconnectInterval = ref(reconnectInterval)
  let reconnectTimer = null
  const eventHandlers = new Map()

  // Get WebSocket URL based on current location
  const getWebSocketUrl = () => {
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    const host = import.meta.env.VITE_WS_HOST || window.location.hostname
    const port = import.meta.env.VITE_WS_PORT || '3005'
    return `${protocol}//${host}:${port}${endpoint}`
  }

  const connect = () => {
    if (connecting.value || connected.value) return

    connecting.value = true
    const url = getWebSocketUrl()
    
    try {
      ws.value = new WebSocket(url)

      ws.value.onopen = (event) => {
        connected.value = true
        connecting.value = false
        reconnectAttempts.value = 0
        reconnectFailed.value = false
        currentReconnectInterval.value = reconnectInterval
        console.log(`WebSocket connected: ${endpoint}`)
        
        if (onOpen) onOpen(event)
        emit('open', event)
      }

      ws.value.onclose = (event) => {
        connected.value = false
        connecting.value = false
        console.log(`WebSocket closed: ${endpoint}`)
        
        if (onClose) onClose(event)
        emit('close', event)

        // Attempt reconnect if enabled
        if (reconnect && !event.wasClean) {
          scheduleReconnect()
        }
      }

      ws.value.onerror = (error) => {
        console.error(`WebSocket error: ${endpoint}`, error)
        
        if (onError) onError(error)
        emit('error', error)
      }

      ws.value.onmessage = (event) => {
        try {
          const data = JSON.parse(event.data)
          
          if (onMessage) onMessage(data)
          
          // Emit to specific event type handlers
          if (data.type) {
            emit(data.type, data)
          }
          
          // Emit generic message event
          emit('message', data)
        } catch (error) {
          console.error('Failed to parse WebSocket message:', error)
        }
      }
    } catch (error) {
      console.error(`Failed to create WebSocket connection: ${endpoint}`, error)
      connecting.value = false
      
      if (reconnect) {
        scheduleReconnect()
      }
    }
  }

  const disconnect = () => {
    if (reconnectTimer) {
      clearTimeout(reconnectTimer)
      reconnectTimer = null
    }

    if (ws.value) {
      ws.value.close()
      ws.value = null
    }

    connected.value = false
    connecting.value = false
  }

  const scheduleReconnect = () => {
    if (reconnectTimer) return

    // Check if max attempts exceeded
    if (reconnectAttempts.value >= maxReconnectAttempts) {
      console.error(
        `❌ Max reconnect attempts (${maxReconnectAttempts}) reached for ${endpoint}. Giving up.`
      )
      reconnectFailed.value = true
      
      if (onReconnectFailed) onReconnectFailed()
      emit('reconnect-failed', { endpoint, attempts: reconnectAttempts.value })
      
      return
    }

    reconnectAttempts.value++
    
    console.log(
      `Reconnecting to ${endpoint} (attempt ${reconnectAttempts.value}/${maxReconnectAttempts}) in ${currentReconnectInterval.value}ms`
    )

    reconnectTimer = setTimeout(() => {
      reconnectTimer = null
      connect()
      
      // Exponential backoff
      currentReconnectInterval.value = Math.min(
        currentReconnectInterval.value * reconnectDecay,
        maxReconnectInterval
      )
    }, currentReconnectInterval.value)
  }

  const send = (data) => {
    if (!connected.value || !ws.value) {
      console.warn('Cannot send message: WebSocket not connected')
      return false
    }

    try {
      const message = typeof data === 'string' ? data : JSON.stringify(data)
      ws.value.send(message)
      return true
    } catch (error) {
      console.error('Failed to send WebSocket message:', error)
      return false
    }
  }

  const on = (event, handler) => {
    if (!eventHandlers.has(event)) {
      eventHandlers.set(event, [])
    }
    eventHandlers.get(event).push(handler)

    // Return unsubscribe function
    return () => {
      const handlers = eventHandlers.get(event)
      if (handlers) {
        const index = handlers.indexOf(handler)
        if (index > -1) {
          handlers.splice(index, 1)
        }
      }
    }
  }

  const emit = (event, data) => {
    const handlers = eventHandlers.get(event)
    if (handlers) {
      handlers.forEach(handler => {
        try {
          handler(data)
        } catch (error) {
          console.error(`Error in WebSocket event handler for ${event}:`, error)
        }
      })
    }
  }

  // Auto-connect on mount if enabled
  if (autoConnect) {
    connect()
  }

  // Manual retry after max attempts exceeded
  const retryConnection = () => {
    console.log(`🔄 Manual retry requested for ${endpoint}`)
    reconnectAttempts.value = 0
    reconnectFailed.value = false
    currentReconnectInterval.value = reconnectInterval
    connect()
  }

  // Cleanup on unmount
  onUnmounted(() => {
    disconnect()
  })

  return {
    ws,
    connected,
    connecting,
    reconnectAttempts,
    reconnectFailed,
    connect,
    disconnect,
    send,
    on,
    retryConnection
  }
}
