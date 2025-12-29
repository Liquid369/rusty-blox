// API Module - Refactored and Modular Structure
//
// This module provides a clean, maintainable API with caching support.
// Each domain (blocks, transactions, addresses, etc.) is in its own submodule.

pub mod types;
pub mod helpers;
pub mod network;
pub mod blocks;
pub mod transactions;
pub mod addresses;
pub mod masternodes;
pub mod governance;
pub mod search;

// Re-export all public items
pub use types::*;
pub use helpers::*;
pub use network::*;
pub use blocks::*;
pub use transactions::*;
pub use addresses::*;
pub use masternodes::*;
pub use governance::*;
pub use search::*;

// Keep root and api handlers for backward compatibility
pub async fn root_handler() -> axum::response::Html<String> {
    match std::fs::read_to_string("frontend/index.html") {
        Ok(html) => axum::response::Html(html),
        Err(_) => axum::response::Html(
            "<h1>Error: Frontend not found</h1><p>Please ensure frontend/index.html exists.</p>".to_string()
        ),
    }
}

pub async fn api_handler() -> &'static str {
    "API response"
}
