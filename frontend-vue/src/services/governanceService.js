import api from './api'

export const governanceService = {
  async getBudgetInfo() {
    const response = await api.get('/v2/budgetinfo')
    return response.data
  },

  async getBudgetVotes(proposalName) {
    const response = await api.get(`/v2/budgetvotes/${encodeURIComponent(proposalName)}`)
    return response.data
  },

  async getBudgetProjection() {
    const response = await api.get('/v2/budgetprojection')
    return response.data
  }
}
