extern crate sha2;

use sha2::{Sha512, Digest};

#[repr(C)]
pub struct sph_blake_big_context {

    buf: [u8; 128],
    ptr: usize,
    H: [u64; 8],
    S: [u64; 4],
    T0: u64,
    T1: u64,
}

extern "C" {
    pub fn HashQuark(pbegin: *const u8, pend: *const u8, output: *mut u8);
}


fn call_hash_quark(data: &[u8]) -> [u8; 32] {
    let data_ptr = data.as_ptr();
    let data_len = data.len();

    let begin = data_ptr as *const _;
    let end = unsafe { data_ptr.add(data_len) as *const _ };

    unsafe { HashQuark(begin, end) }
}

pub fn sha512_hash(input: &[u8]) -> [u8; 64] {
    let mut hasher = Sha512::new();
    hasher.update(input);
    let result = hasher.finalize();
    let mut hash = [0u8; 64];
    hash.copy_from_slice(&result);
    hash
}
