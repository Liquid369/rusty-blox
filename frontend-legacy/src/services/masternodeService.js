import api from './api'

export const masternodeService = {
  /**
   * Get masternode count statistics
   */
  async getMasternodeCount() {
    const response = await api.get('/api/v2/mncount')
    return response.data
  },

  /**
   * Get full masternode list
   */
  async getMasternodeList() {
    const response = await api.get('/api/v2/mnlist')
    return response.data
  },

  /**
   * Relay masternode broadcast message
   */
  async relayMasternodeBroadcast(hexData) {
    const response = await api.get(`/api/v2/relaymnb/${hexData}`)
    return response.data
  }
}
