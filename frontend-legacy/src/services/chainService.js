import api from './api'

export const chainService = {
  /**
   * Get current chain status
   */
  async getStatus() {
    const response = await api.get('/api/v2/status')
    return response.data
  },

  /**
   * Get chain statistics
   */
  async getStats() {
    // This would be a dedicated stats endpoint
    // For now, use status endpoint
    return this.getStatus()
  }
}

export default chainService
