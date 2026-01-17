import api from './api'

export const governanceService = {
  /**
   * Get all budget proposals
   */
  async getBudgetInfo() {
    const response = await api.get('/api/v2/budgetinfo')
    return response.data
  },

  /**
   * Get votes for a specific proposal
   */
  async getBudgetVotes(proposalName) {
    const response = await api.get(`/api/v2/budgetvotes/${proposalName}`)
    return response.data
  },

  /**
   * Get budget projection
   */
  async getBudgetProjection() {
    const response = await api.get('/api/v2/budgetprojection')
    return response.data
  }
}
