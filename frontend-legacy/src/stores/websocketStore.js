import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { useWebSocket } from '@/composables/useWebSocket'

/**
 * WebSocket store for managing real-time connections
 * Provides centralized WebSocket management for blocks, transactions, and mempool
 */
export const useWebSocketStore = defineStore('websocket', () => {
  // Connection instances
  const blockConnection = ref(null)
  const transactionConnection = ref(null)
  const mempoolConnection = ref(null)

  // Connection states
  const blockConnected = ref(false)
  const transactionConnected = ref(false)
  const mempoolConnected = ref(false)
  
  // Reconnect failure states
  const blockReconnectFailed = ref(false)
  const transactionReconnectFailed = ref(false)
  const mempoolReconnectFailed = ref(false)

  // Event data
  const latestBlock = ref(null)
  const latestTransaction = ref(null)
  const mempoolSize = ref(0)

  // Event handlers storage
  const blockHandlers = ref([])
  const transactionHandlers = ref([])
  const mempoolHandlers = ref([])

  // Computed overall connection status
  const anyConnected = computed(() => 
    blockConnected.value || transactionConnected.value || mempoolConnected.value
  )

  const allConnected = computed(() => 
    blockConnected.value && transactionConnected.value && mempoolConnected.value
  )
  
  const anyReconnectFailed = computed(() =>
    blockReconnectFailed.value || transactionReconnectFailed.value || mempoolReconnectFailed.value
  )

  /**
   * Initialize block WebSocket connection
   */
  const connectBlocks = () => {
    if (blockConnection.value) return

    const ws = useWebSocket('/ws/blocks', {
      autoConnect: true,
      reconnect: true,
      maxReconnectAttempts: 20,
      onOpen: () => {
        blockConnected.value = true
        blockReconnectFailed.value = false
        console.log('Block WebSocket connected')
      },
      onClose: () => {
        blockConnected.value = false
        console.log('Block WebSocket disconnected')
      },
      onReconnectFailed: () => {
        blockReconnectFailed.value = true
        console.error('Block WebSocket reconnect failed after max attempts')
      },
      onMessage: (data) => {
        if (data.type === 'NewBlock') {
          latestBlock.value = data
          // Notify all registered handlers
          blockHandlers.value.forEach(handler => {
            try {
              handler(data)
            } catch (error) {
              console.error('Error in block handler:', error)
            }
          })
        }
      }
    })

    blockConnection.value = ws
  }

  /**
   * Initialize transaction WebSocket connection
   */
  const connectTransactions = () => {
    if (transactionConnection.value) return

    const ws = useWebSocket('/ws/transactions', {
      autoConnect: true,
      reconnect: true,
      onOpen: () => {
        transactionConnected.value = true
        console.log('Transaction WebSocket connected')
      },
      onClose: () => {
        transactionConnected.value = false
        console.log('Transaction WebSocket disconnected')
      },
      onMessage: (data) => {
        if (data.type === 'NewTransaction') {
          latestTransaction.value = data
          // Notify all registered handlers
          transactionHandlers.value.forEach(handler => {
            try {
              handler(data)
            } catch (error) {
              console.error('Error in transaction handler:', error)
            }
          })
        }
      }
    })

    transactionConnection.value = ws
  }

  /**
   * Initialize mempool WebSocket connection
   */
  const connectMempool = () => {
    if (mempoolConnection.value) return

    const ws = useWebSocket('/ws/mempool', {
      autoConnect: true,
      reconnect: true,
      onOpen: () => {
        mempoolConnected.value = true
        console.log('Mempool WebSocket connected')
      },
      onClose: () => {
        mempoolConnected.value = false
        console.log('Mempool WebSocket disconnected')
      },
      onMessage: (data) => {
        if (data.type === 'MempoolUpdate') {
          // Update mempool size based on action
          if (data.action === 'added') {
            mempoolSize.value++
          } else if (data.action === 'removed') {
            mempoolSize.value = Math.max(0, mempoolSize.value - 1)
          }
          
          // Notify all registered handlers
          mempoolHandlers.value.forEach(handler => {
            try {
              handler(data)
            } catch (error) {
              console.error('Error in mempool handler:', error)
            }
          })
        }
      }
    })

    mempoolConnection.value = ws
  }

  /**
   * Connect to all WebSocket endpoints
   */
  const connectAll = () => {
    connectBlocks()
    connectTransactions()
    connectMempool()
  }

  /**
   * Disconnect from specific endpoint
   */
  const disconnectBlocks = () => {
    if (blockConnection.value) {
      blockConnection.value.disconnect()
      blockConnection.value = null
      blockConnected.value = false
    }
  }

  const disconnectTransactions = () => {
    if (transactionConnection.value) {
      transactionConnection.value.disconnect()
      transactionConnection.value = null
      transactionConnected.value = false
    }
  }

  const disconnectMempool = () => {
    if (mempoolConnection.value) {
      mempoolConnection.value.disconnect()
      mempoolConnection.value = null
      mempoolConnected.value = false
    }
  }

  /**
   * Disconnect from all WebSocket endpoints
   */
  const disconnectAll = () => {
    disconnectBlocks()
    disconnectTransactions()
    disconnectMempool()
  }

  /**
   * Register event handler for new blocks
   * @param {function} handler - Callback function to handle block events
   * @returns {function} Unsubscribe function
   */
  const onNewBlock = (handler) => {
    blockHandlers.value.push(handler)
    
    // Return unsubscribe function
    return () => {
      const index = blockHandlers.value.indexOf(handler)
      if (index > -1) {
        blockHandlers.value.splice(index, 1)
      }
    }
  }

  /**
   * Register event handler for new transactions
   * @param {function} handler - Callback function to handle transaction events
   * @returns {function} Unsubscribe function
   */
  const onNewTransaction = (handler) => {
    transactionHandlers.value.push(handler)
    
    return () => {
      const index = transactionHandlers.value.indexOf(handler)
      if (index > -1) {
        transactionHandlers.value.splice(index, 1)
      }
    }
  }

  /**
   * Register event handler for mempool updates
   * @param {function} handler - Callback function to handle mempool events
   * @returns {function} Unsubscribe function
   */
  const onMempoolUpdate = (handler) => {
    mempoolHandlers.value.push(handler)
    
    return () => {
      const index = mempoolHandlers.value.indexOf(handler)
      if (index > -1) {
        mempoolHandlers.value.splice(index, 1)
      }
    }
  }

  // Manual retry methods
  const retryBlocks = () => {
    if (blockConnection.value) {
      blockConnection.value.retryConnection()
    } else {
      connectBlocks()
    }
  }
  
  const retryAll = () => {
    if (blockReconnectFailed.value) retryBlocks()
    // Add more retry calls here if transaction/mempool also fail
  }

  return {
    // Connections
    blockConnection,
    transactionConnection,
    mempoolConnection,

    // Connection states
    blockConnected,
    transactionConnected,
    mempoolConnected,
    blockReconnectFailed,
    transactionReconnectFailed,
    mempoolReconnectFailed,
    anyConnected,
    allConnected,
    anyReconnectFailed,

    // Latest data
    latestBlock,
    latestTransaction,
    mempoolSize,

    // Connection methods
    connectBlocks,
    connectTransactions,
    connectMempool,
    connectAll,
    disconnectBlocks,
    disconnectTransactions,
    disconnectMempool,
    disconnectAll,

    // Retry methods
    retryBlocks,
    retryAll,

    // Event registration
    onNewBlock,
    onNewTransaction,
    onMempoolUpdate
  }
})
