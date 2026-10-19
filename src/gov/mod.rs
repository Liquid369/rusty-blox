//! Governance vote history. Core keeps one vote per masternode per proposal,
//! overwrites it in place, and deletes expired proposals with their votes, so
//! history exists only if it is polled and stored here.

pub mod diff;
pub mod live;
pub mod poller;
pub mod store;
