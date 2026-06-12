import api from './api'

export const blockService = {
  async getRecentBlocks(limit = 10) {
    // Use block-stats endpoint for recent blocks
    const response = await api.get(`/v2/block-stats/${limit}`)
    return response.data
  },

  async getBlock(id) {
    // Get block detail by height or hash
    const response = await api.get(`/v2/block-detail/${id}`)
    return response.data
  },

  async getBlockTransactions(id) {
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

  // Fetch multiple transactions in parallel, preserving input order.
  // Failed lookups are skipped.
  async getTransactions(txids) {
    const promises = txids.map((txid, index) =>
      this.getTransaction(txid).then((data) => ({ index, data }))
    )
    const results = await Promise.allSettled(promises)
    return results
      .filter((result) => result.status === 'fulfilled')
      .map((result) => result.value)
      .sort((a, b) => a.index - b.index)
      .map((item) => item.data)
  }
}

export const addressService = {
  async getAddress(address, page = 1, pageSize = 25) {
    const response = await api.get(`/v2/address/${address}`, {
      params: { page, pageSize }
    })
    return response.data
  },

  async getAddressUtxos(address) {
    const response = await api.get(`/v2/utxo/${address}`)
    return response.data
  }
}

export const xpubService = {
  async getXPub(xpub, params = {}) {
    const response = await api.get(`/v2/xpub/${xpub}`, { params })
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
  },

  async getMoneySupply() {
    const response = await api.get('/v2/moneysupply')
    return response.data
  }
}

export const masternodeService = {
  async getMasternodeCount() {
    const response = await api.get('/v2/mncount')
    return response.data
  }
}

export const mempoolService = {
  async getMempool() {
    const response = await api.get('/v2/mempool')
    return response.data
  },

  async getMempoolTransaction(txid) {
    const response = await api.get(`/v2/mempool/${txid}`)
    return response.data
  }
}

export const searchService = {
  async search(query) {
    const trimmed = query.trim()
    if (!trimmed) {
      throw new Error('Search query cannot be empty')
    }
    try {
      const response = await api.get(`/v2/search/${encodeURIComponent(trimmed)}`)
      return response.data
    } catch (error) {
      if (error.response && error.response.status === 404) {
        return { type: 'NotFound', query: trimmed }
      }
      throw error
    }
  }
}
