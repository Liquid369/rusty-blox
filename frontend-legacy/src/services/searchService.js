import api from './api'

export const searchService = {
  /**
   * Universal search - detects and searches for blocks, transactions, or addresses
   */
  async search(query) {
    const trimmedQuery = query.trim()
    
    if (!trimmedQuery) {
      throw new Error('Search query cannot be empty')
    }
    
    try {
      const response = await api.get(`/api/v2/search/${encodeURIComponent(trimmedQuery)}`)
      return response.data
    } catch (error) {
      if (error.response && error.response.status === 404) {
        return {
          type: 'NotFound',
          query: trimmedQuery,
          message: 'No results found'
        }
      }
      throw error
    }
  },

  /**
   * Get search suggestions based on partial query
   */
  async getSuggestions(query) {
    // This would require a backend endpoint for autocomplete
    // For now, return empty array
    return []
  }
}

export default searchService
