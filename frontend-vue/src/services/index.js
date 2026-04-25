import api from './api'

export const blockService = {
  async getRecentBlocks(limit = 10, offset = 0) {
    // Use block-stats endpoint for recent blocks
    const response = await api.get(`/v2/block-stats/${limit}`)
    return response.data
  },

  async getBlock(id) {
    // Get block detail by height or hash
    const response = await api.get(`/v2/block-detail/${id}`)
    return response.data
  },

  async getBlockTransactions(id, limit = 25, offset = 0) {
    // Block detail includes transactions
    const response = await api.get(`/v2/block-detail/${id}`)
    return response.data
  }
}

export const transactionService = {
  async getTransaction(txid) {
    const response = await api.get(`/v2/tx/${txid}`)
    return response.data
  },

  async getRecentTransactions(limit = 10, offset = 0) {
    // Get from mempool or recent blocks
    const response = await api.get('/v2/mempool')
    return response.data
  }
}

export const addressService = {
  async getAddress(address) {
    const response = await api.get(`/v2/address/${address}`)
    return response.data
  },

  async getAddressTransactions(address, limit = 25, offset = 0) {
    const response = await api.get(`/v2/address/${address}`, {
      params: { limit, offset }
    })
    return response.data
  },

  async getAddressUtxos(address) {
    const response = await api.get(`/v2/utxo/${address}`)
    return response.data
  }
}

export const chainService = {
  async getStatus() {
    const response = await api.get('/v2/status')
    return response.data
  },

  async getChainInfo() {
    const response = await api.get('/v2/status')
    return response.data
  }
}

export const searchService = {
  async search(query) {
    const response = await api.get(`/v2/search/${query}`)
    return response.data
  }
}

