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
pub mod analytics;

#[cfg(test)]
mod xpub_tests;

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
pub use analytics::*;

// Keep root and api handlers for backward compatibility
pub async fn root_handler() -> axum::response::Html<String> {
    // Try multiple frontend locations in priority order:
    // 1. Production build (frontend-vue/dist)
    // 2. Legacy frontend (frontend or frontend-legacy)
    let frontend_paths = [
        "frontend-vue/dist/index.html",
        "frontend/index.html",
        "frontend-legacy/index.html",
    ];
    
    for path in &frontend_paths {
        if let Ok(html) = std::fs::read_to_string(path) {
            return axum::response::Html(html);
        }
    }
    
    axum::response::Html(
        r#"<h1>Error: Frontend not found</h1>
<p>Please ensure one of the following exists:</p>
<ul>
  <li><code>frontend-vue/dist/index.html</code> (production build: <code>cd frontend-vue && npm run build</code>)</li>
  <li><code>frontend/index.html</code> (legacy frontend)</li>
</ul>
<p>For development, run the Vue frontend separately: <code>cd frontend-vue && npm run dev</code></p>"#.to_string()
    )
}

pub async fn api_handler() -> &'static str {
    "API response"
}
