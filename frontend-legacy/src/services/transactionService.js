import api from './api'

export const transactionService = {
  /**
   * Get transaction details by txid
   */
  async getTransaction(txid) {
    const response = await api.get(`/api/v2/tx/${txid}`)
    return response.data
  },

  /**
   * Get multiple transactions by txids
   * Preserves the input order of txids in the output
   */
  async getTransactions(txids) {
    const promises = txids.map((txid, index) => 
      this.getTransaction(txid).then(data => ({ index, data }))
    )
    const results = await Promise.allSettled(promises)
    
    // Sort by original index to preserve order
    return results
      .filter(result => result.status === 'fulfilled')
      .map(result => result.value)
      .sort((a, b) => a.index - b.index)
      .map(item => item.data)
  }
}

export default transactionService
