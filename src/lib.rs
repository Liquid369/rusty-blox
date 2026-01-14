extern crate sha2;

use sha2::{Sha512, Digest};
use std::ffi::c_void;

pub mod chainwork;
pub mod canonical_chain;
pub mod config;
pub mod constants;
pub mod db_handles;
pub mod tx_keys;
pub mod enrich_addresses;
pub mod enrich_from_chainstate;
pub mod address;
pub mod parallel;
pub mod monitor;
pub mod websocket;
pub mod chain_state;
pub mod cache;
pub mod blocks;
pub mod db_utils;
pub mod batch_writer;
pub mod atomic_writer;
pub mod transactions;
pub mod leveldb_index;
pub mod block_index;
pub mod offset_indexer;
pub mod pivx_copy;
pub mod api;
pub mod mempool;
pub mod block_detail;
pub mod chainstate_leveldb;
pub mod chainstate;
pub mod script_utils;
pub mod utxo;
pub mod forks;
pub mod sync;
pub mod parser;
pub mod search;
pub mod repair;
pub mod height_resolver;
pub mod types;
pub mod tx_type;
pub mod maturity;
pub mod reorg;
pub mod spent_utxo;
pub mod pos_validation;
pub mod address_rollback;
pub mod build_address_undo;
pub mod fee_calculation;
pub mod script_validation;
pub mod sapling_validation;

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