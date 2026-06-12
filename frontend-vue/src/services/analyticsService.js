import api from './api'

export const analyticsService = {
  async getTransactionAnalytics(range = '30d') {
    const response = await api.get('/v2/analytics/transactions', {
      params: { range }
    })
    return response.data
  },

  async getStakingAnalytics(range = '30d') {
    const response = await api.get('/v2/analytics/staking', {
      params: { range }
    })
    return response.data
  },

  async getSupplyAnalytics() {
    const response = await api.get('/v2/analytics/supply')
    return response.data
  },

  async getNetworkHealth(range = '30d') {
    const response = await api.get('/v2/analytics/network', {
      params: { range }
    })
    return response.data
  },

  async getRichList(limit = 100) {
    const response = await api.get('/v2/analytics/richlist', {
      params: { limit }
    })
    return response.data
  },

  async getWealthDistribution() {
    const response = await api.get('/v2/analytics/wealth-distribution')
    return response.data
  }
}
