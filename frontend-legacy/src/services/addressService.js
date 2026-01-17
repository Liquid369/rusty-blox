import api from './api'

export const addressService = {
  /**
   * Get address details including balance and transactions
   * @param {string} address - The address to fetch
   * @param {number} page - Page number for pagination
   * @returns {Promise<Object>} Address data
   */
  async getAddress(address, page = 1) {
    const response = await api.get(`/api/v2/address/${address}`, {
      params: { 
        page,
        _cb: Date.now() // Cache-busting parameter
      }
    })
    return response.data
  },

  /**
   * Get UTXOs for an address
   * @param {string} address - The address to fetch UTXOs for
   * @returns {Promise<Array>} Array of UTXOs
   */
  async getUTXOs(address) {
    const response = await api.get(`/api/v2/utxo/${address}`)
    return response.data
  },

  /**
   * Get balance for an address
   * @param {string} address - The address to fetch balance for
   * @returns {Promise<Object>} Balance information
   */
  async getBalance(address) {
    const response = await api.get(`/api/v2/address/${address}`)
    return {
      balance: response.data.balance,
      totalReceived: response.data.totalReceived,
      totalSent: response.data.totalSent,
      txCount: response.data.txs
    }
  }
}
