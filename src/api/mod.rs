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
pub mod price;

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
pub use price::*;

// Degraded "/" fallback. main.rs only routes here when the SPA ServeDir did
// NOT find `{paths.frontend_dist}/index.html` at startup, so we re-read the SAME
// configured build path at request time (it may have been built after launch)
// and serve nothing else. We deliberately do not probe other directories
// (e.g. an older frontend-vue/dist): silently serving a stale UI from a
// different build masks a missing/mislocated dist and ships the wrong frontend.
pub async fn root_handler() -> axum::response::Html<String> {
    let frontend_dist = crate::config::get_global_config()
        .get_string("paths.frontend_dist")
        .unwrap_or_else(|_| "frontend-legacy/dist".to_string());
    let index_path = std::path::Path::new(&frontend_dist).join("index.html");

    if let Ok(html) = std::fs::read_to_string(&index_path) {
        return axum::response::Html(html);
    }

    axum::response::Html(format!(
        r#"<h1>Error: Frontend not found</h1>
<p>No production build at <code>{}</code>.</p>
<p>Build it and restart the service from the repository root (so the relative
path resolves), or set <code>paths.frontend_dist</code> to an absolute path:</p>
<pre>cd frontend-legacy &amp;&amp; npm ci &amp;&amp; npm run build</pre>"#,
        index_path.display()
    ))
}

pub async fn api_handler() -> &'static str {
    "API response"
}
