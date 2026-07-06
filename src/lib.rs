extern crate sha2;

use sha2::{Digest, Sha512};
use std::ffi::c_void;

/// The authoritative set of RocksDB column families this database creates and
/// opens, in ONE place so every read-write open path stays in sync. The "default"
/// CF is implicit and handled separately by each open site.
///
/// Read-WRITE opens must list every on-disk CF (RocksDB requirement), so they
/// either use this constant (creator paths: the main binary, import-chainstate,
/// resync) or `DB::list_cf` dynamic discovery (occasional maintenance tools).
/// Read-only opens may list a subset.
///
/// `tail_blocks` / `tail_meta` are private to the opt-in live orphan-tail feature
/// (see DESIGN-live-orphan-capture.md); they are created empty and stay empty
/// when `sync.live_tail_blkfiles` is off.
pub const COLUMN_FAMILIES: &[&str] = &[
    "blocks",
    "transactions",
    "addr_index",
    "utxo",
    "chain_metadata",
    "pubkey",
    "chain_state",
    "utxo_undo",   // spent-UTXO tracking for reorg handling + input-value calc
    "tail_blocks", // private: live orphan-tail block records (opt-in)
    "tail_meta",   // private: tail cursor/anchor + (claimed_height,state) index
];

pub mod address;
pub mod atomic_writer;
pub mod batch_writer;
pub mod blocks;
pub mod cache;
pub mod canonical_chain;
pub mod chain_state;
pub mod chainwork;
pub mod config;
pub mod constants;
pub mod db_handles;
pub mod db_sampler;
pub mod db_utils;
pub mod emission;
pub mod enrich_addresses;
pub mod enrich_from_chainstate;
pub mod monitor;
pub mod parallel;
pub mod transactions;
pub mod tx_keys;
pub mod websocket;

pub mod address_rollback;
pub mod analytics_live;
pub mod analytics_recompute;
pub mod api;
pub mod blk_tail;
pub mod block_detail;
pub mod block_index;
pub mod build_address_undo;
pub mod chainstate;
pub mod chainstate_leveldb;
pub mod crash_recovery;
pub mod fee_calculation;
pub mod forks;
pub mod height_resolver;
pub mod leveldb_index;
pub mod maturity;
pub mod mempool;
pub mod metrics;
pub mod offset_indexer;
pub mod parser;
pub mod pivx_copy;
pub mod reorg;
pub mod repair;
pub mod script_utils;
pub mod search;
pub mod spent_utxo;
pub mod sync;
pub mod telemetry;
pub mod tx_type;
pub mod types;
pub mod utxo;

#[repr(C)]
#[allow(non_snake_case)] // C FFI struct - must match quark hash C implementation
pub struct sph_blake_big_context {
    buf: [u8; 128],
    ptr: usize,
    H: [u64; 8],
    S: [u64; 4],
    T0: u64,
    T1: u64,
}

extern "C" {
    pub fn quark_hash(input: *const u8, output: *mut u8, len: u32);
    pub fn sph_blake512_init(cc: *mut c_void);
    pub fn sph_blake512(cc: *mut c_void, data: *const c_void, len: usize);
    pub fn sph_blake512_close(cc: *mut c_void, dst: *mut c_void);

    pub fn sph_bmw512_init(cc: *mut c_void);
    pub fn sph_bmw512(cc: *mut c_void, data: *const c_void, len: usize);
    pub fn sph_bmw512_close(cc: *mut c_void, dst: *mut c_void);

    pub fn sph_groestl512_init(cc: *mut c_void);
    pub fn sph_groestl512(cc: *mut c_void, data: *const c_void, len: usize);
    pub fn sph_groestl512_close(cc: *mut c_void, dst: *mut c_void);

    pub fn sph_skein512_init(cc: *mut c_void);
    pub fn sph_skein512(cc: *mut c_void, data: *const c_void, len: usize);
    pub fn sph_skein512_close(cc: *mut c_void, dst: *mut c_void);

    pub fn sph_jh512_init(cc: *mut c_void);
    pub fn sph_jh512(cc: *mut c_void, data: *const c_void, len: usize);
    pub fn sph_jh512_close(cc: *mut c_void, dst: *mut c_void);

    pub fn sph_keccak512_init(cc: *mut c_void);
    pub fn sph_keccak512(cc: *mut c_void, data: *const c_void, len: usize);
    pub fn sph_keccak512_close(cc: *mut c_void, dst: *mut c_void);
}

pub fn call_quark_hash(data: &[u8]) -> [u8; 32] {
    let mut output_hash = [0u8; 32]; // Buffer for the hash result

    unsafe {
        quark_hash(data.as_ptr(), output_hash.as_mut_ptr(), data.len() as u32);
    }

    output_hash
}

pub fn sha512_hash(input: &[u8]) -> [u8; 64] {
    let mut hasher = Sha512::new();
    hasher.update(input);
    let result = hasher.finalize();
    let mut hash = [0u8; 64];
    hash.copy_from_slice(&result);
    hash
}
