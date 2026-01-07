import api from './api'

export const analyticsService = {
  /**
   * Get supply analytics data
   * @param {string} timeRange - Time range (24h, 7d, 30d, 90d, 1y, all)
   * @returns {Promise<Object>} Supply data
   */
  async getSupplyAnalytics(timeRange = '30d') {
    const response = await api.get('/api/v2/analytics/supply', {
      params: { range: timeRange }
    })
    return response.data
  },

  /**
   * Get transaction analytics data
   * @param {string} timeRange - Time range
   * @returns {Promise<Object>} Transaction analytics
   */
  async getTransactionAnalytics(timeRange = '30d') {
    const response = await api.get('/api/v2/analytics/transactions', {
      params: { range: timeRange }
    })
    return response.data
  },

  /**
   * Get staking analytics data
   * @param {string} timeRange - Time range
   * @returns {Promise<Object>} Staking data
   */
  async getStakingAnalytics(timeRange = '30d') {
    const response = await api.get('/api/v2/analytics/staking', {
      params: { range: timeRange }
    })
    return response.data
  },

  /**
   * Get network health metrics
   * @param {string} timeRange - Time range
   * @returns {Promise<Object>} Network health data
   */
  async getNetworkHealth(timeRange = '30d') {
    const response = await api.get('/api/v2/analytics/network', {
      params: { range: timeRange }
    })
    return response.data
  },

  /**
   * Get rich list (top addresses by balance)
   * @param {number} limit - Number of addresses to return
   * @returns {Promise<Array>} Top addresses
   */
  async getRichList(limit = 100) {
    const response = await api.get('/api/v2/analytics/richlist', {
      params: { limit }
    })
    return response.data
  },

  /**
   * Get wealth distribution data
   * @returns {Promise<Object>} Wealth distribution
   */
  async getWealthDistribution() {
    const response = await api.get('/api/v2/analytics/wealth-distribution')
    return response.data
  }
}
