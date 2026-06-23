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
   * Get multiple transactions by txids.
   *
   * Resolves details with a bounded concurrency window so a page of txids
   * never fans out into an unbounded burst of simultaneous requests
   * (prevents browser jank and hammering the backend). The output preserves
   * the input order of txids; failed lookups are dropped.
   *
   * @param {string[]} txids
   * @param {number} concurrency - max in-flight requests (default: 6)
   */
  async getTransactions(txids, concurrency = 6) {
    if (!Array.isArray(txids) || txids.length === 0) return []

    const results = new Array(txids.length)
    const limit = Math.max(1, Math.min(concurrency, txids.length))
    let cursor = 0

    const worker = async () => {
      while (cursor < txids.length) {
        const index = cursor++
        try {
          results[index] = await this.getTransaction(txids[index])
        } catch {
          results[index] = null
        }
      }
    }

    await Promise.all(Array.from({ length: limit }, worker))

    // Preserve input order, drop failed lookups
    return results.filter(tx => tx != null)
  }
}

export default transactionService
