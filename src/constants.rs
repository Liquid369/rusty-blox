/// Block and Transaction Height Constants
/// 
/// These constants ensure consistent handling of special height values across the codebase.
/// All height-related logic should use these constants instead of magic numbers.

/// Genesis block height (the first block in the chain)
pub const HEIGHT_GENESIS: i32 = 0;

/// Orphan block marker - blocks that exist in blk files but are not on the canonical chain
/// This includes:
/// - Blocks from abandoned forks
/// - Blocks that lost in a reorg
/// - Blocks that were never part of the main chain
pub const HEIGHT_ORPHAN: i32 = -1;

/// Unresolved height marker - temporary marker during processing
/// Used when:
/// - Block is being processed but height not yet determined
/// - Race condition during sync (should be fixed by metadata validation)
/// - Awaiting enrichment phase to resolve height
/// 
/// Note: After Fix #1, this should rarely appear. If it does, it indicates
/// blocks that aren't in canonical chain metadata (likely orphans).
pub const HEIGHT_UNRESOLVED: i32 = -2;

/// Check if a height represents a valid canonical block
#[inline]
pub fn is_canonical_height(height: i32) -> bool {
    height >= HEIGHT_GENESIS
}

/// Check if a height represents an orphan block
#[inline]
pub fn is_orphan_height(height: i32) -> bool {
    height == HEIGHT_ORPHAN
}

/// Check if a height represents an unresolved block
#[inline]
pub fn is_unresolved_height(height: i32) -> bool {
    height == HEIGHT_UNRESOLVED
}

/// Check if a height is the genesis block
#[inline]
pub fn is_genesis_height(height: i32) -> bool {
    height == HEIGHT_GENESIS
}

/// Check if a transaction should be indexed in address enrichment
/// 
/// Returns true for:
/// - Genesis block (height = 0)
/// - All canonical blocks (height > 0)
/// 
/// Returns false for:
/// - Orphan blocks (height = -1)
/// - Unresolved blocks (height = -2)
#[inline]
pub fn should_index_transaction(height: i32) -> bool {
    is_canonical_height(height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_height_constants() {
        assert_eq!(HEIGHT_GENESIS, 0);
        assert_eq!(HEIGHT_ORPHAN, -1);
        assert_eq!(HEIGHT_UNRESOLVED, -2);
    }

    #[test]
    fn test_canonical_height() {
        assert!(is_canonical_height(0));
        assert!(is_canonical_height(1));
        assert!(is_canonical_height(1000000));
        assert!(!is_canonical_height(-1));
        assert!(!is_canonical_height(-2));
    }

    #[test]
    fn test_orphan_height() {
        assert!(is_orphan_height(-1));
        assert!(!is_orphan_height(0));
        assert!(!is_orphan_height(-2));
    }

    #[test]
    fn test_unresolved_height() {
        assert!(is_unresolved_height(-2));
        assert!(!is_unresolved_height(0));
        assert!(!is_unresolved_height(-1));
    }

    #[test]
    fn test_genesis_height() {
        assert!(is_genesis_height(0));
        assert!(!is_genesis_height(1));
        assert!(!is_genesis_height(-1));
    }

    #[test]
    fn test_should_index() {
        // Should index genesis and canonical blocks
        assert!(should_index_transaction(0));
        assert!(should_index_transaction(1));
        assert!(should_index_transaction(1000000));
        
        // Should NOT index orphans or unresolved
        assert!(!should_index_transaction(-1));
        assert!(!should_index_transaction(-2));
    }
}
