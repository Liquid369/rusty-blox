import api from './api'

export const blockService = {
  /**
   * Get block by height or hash
   */
  async getBlock(identifier) {
    const response = await api.get(`/api/v2/block/${identifier}`)
    return response.data
  },

  /**
   * Get detailed block information with transactions
   */
  async getBlockDetail(height) {
    const response = await api.get(`/api/v2/block/${height}`)
    return response.data
  },

  /**
   * Get recent blocks with parallel fetching
   * Returns {blocks, errors} structure for error transparency
   */
  async getRecentBlocks(count = 20) {
    // Get current height first
    const statusResponse = await api.get('/api/v2/status')
    const currentHeight = statusResponse.data.network_height
    
    // Generate heights array
    const heights = []
    for (let i = 0; i < count; i++) {
      const height = currentHeight - i
      if (height >= 0) heights.push(height)
    }
    
    // Fetch all blocks in parallel
    const results = await Promise.allSettled(
      heights.map(height => this.getBlock(height))
    )
    
    // Separate successful blocks from errors
    const blocks = []
    const errors = []
    
    results.forEach((result, index) => {
      if (result.status === 'fulfilled') {
        blocks.push(result.value)
      } else {
        const height = heights[index]
        console.error(`Failed to fetch block ${height}:`, result.reason)
        errors.push({ height, error: result.reason?.message || 'Unknown error' })
      }
    })
    
    // For backward compatibility, return just blocks array if no errors
    // But include errors property for components that want visibility
    return Object.assign(blocks, { errors })
  },

  /**
   * Get blocks in a range
   */
  async getBlockRange(startHeight, endHeight) {
    const blocks = []
    for (let height = startHeight; height <= endHeight; height++) {
      try {
        const block = await this.getBlock(height)
        blocks.push(block)
      } catch (error) {
        console.error(`Failed to fetch block ${height}:`, error)
      }
    }
    return blocks
  }
}

export default blockService
