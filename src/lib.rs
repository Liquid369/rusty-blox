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
    pub fn quark_hash(input: *const u8, output: *mut u8, len: u32);
}

fn call_quark_hash(data: &[u8]) -> [u8; 32] {
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