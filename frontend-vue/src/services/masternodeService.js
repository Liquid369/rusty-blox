import api from './api'

export const masternodeService = {
  async getMasternodeCount() {
    const response = await api.get('/v2/mncount')
    return response.data
  },

  async getMasternodeList() {
    const response = await api.get('/v2/mnlist')
    return response.data
  }
}
