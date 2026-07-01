//! Live orphan discovery by tailing PIVX Core's `blk*.dat` files.
//!
//! This is a PURE OBSERVER: it reads every block Core persists to disk (including
//! side-branch / stale blocks the RPC tip-poll never reports) into two PRIVATE
//! column families (`tail_blocks`, `tail_meta`) and makes ZERO writes to any
//! canonical CF. Core/RPC remains the sole canonical-chain authority.
//!
//! See `DESIGN-live-orphan-capture.md` for the full design (v6). This file is
//! built in sub-steps; 3a here is the framing reader + constants + flag scaffold.
//!
//! On-disk record framing (Core, verified against `master @ 2e9ce17`):
//! ```text
//! [ 4-byte network magic ][ 4-byte size (u32 LE) ][ <size> bytes serialized block ]
//! ```
//! Blocks never straddle files; padding ahead of the write frontier reads as
//! zeros (Linux `posix_fallocate`); there is no per-block fsync, so the trailing
//! record can be torn / not-yet-flushed. We therefore frame strictly on the magic
//! and never trust file length.

use rocksdb::{Direction, IteratorMode, ReadOptions, WriteBatch, DB};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tracing::{info, warn};

type TailResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// PIVX **mainnet** network message-start bytes (`pchMessageStart`).
/// Testnet `F5 E6 D5 CA` / regtest `A1 CF 7E AC` differ — re-confirm against the
/// deployed node if ever run on another network.
pub const PIVX_MAGIC: [u8; 4] = [0x90, 0xc4, 0xfd, 0xe9];

// ---- §9 pinned constants (provisional; tune in Phase 0) -------------------

/// Confirmation depth past which a block's canonical/orphan status is settled.
pub const K_CONFIRM: i32 = 100;
/// Conservative upper bound on observed PIVX reorg depth.
pub const EXPECTED_REORG_DEPTH: i32 = 100;
/// Monitor-stall tolerance, in blocks.
pub const MAX_MONITOR_LAG: i32 = 60;
/// Retention horizon: records below `tip - K` may be evicted. Must exceed the
/// deepest reorg + monitor lag we expect to handle exactly.
pub const K_RETENTION: i32 = 300; // max(EXPECTED_REORG_DEPTH, MAX_MONITOR_LAG) + K_CONFIRM + margin
/// A pending record older than this (seconds, wall-clock since first_seen) is
/// finalized to the terminal `unresolved` state.
pub const MAX_PENDING_AGE_SECS: u64 = 600;
/// Reconcile pass cadence (seconds) — monitor-independent timer.
pub const RECONCILE_INTERVAL_SECS: u64 = 30;
/// Hard ceiling on `tail_blocks` to bound memory under an indefinite monitor
/// stall (frozen eviction). On reaching it the tail PAUSEs ingest and auto-resumes.
pub const MAX_TAIL_BLOCKS: u64 = 1_000_000;
/// Framing sanity bounds for the record size field (bytes).
pub const MIN_BLK: u32 = 80; // a header floor; real blocks are larger
pub const MAX_BLK: u32 = 4 * 1024 * 1024; // well above any PIVX block

/// Config key (under `[sync]`) gating the whole feature. Default off.
pub const FLAG_KEY: &str = "sync.live_tail_blkfiles";

/// Is the live blk-file tail enabled? Reads `sync.live_tail_blkfiles` (default false).
pub fn is_enabled() -> bool {
    crate::config::get_global_config()
        .get_bool(FLAG_KEY)
        .unwrap_or(false)
}

// ---- framing reader -------------------------------------------------------

/// Outcome of attempting to read one block record at a byte offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordRead {
    /// A complete record was parsed. `block` is the serialized block bytes
    /// (header + txs) WITHOUT the 8-byte magic+size frame. `next_offset` is the
    /// absolute byte offset immediately after this record.
    Block { block: Vec<u8>, next_offset: u64 },
    /// The next 4 bytes are all-zero: pre-allocated padding ahead of Core's write
    /// frontier (Q2). Stop reading this file this tick; retry next tick.
    Padding,
    /// The frame header is present but the full body is not on disk yet (torn /
    /// not-yet-flushed write, Q3). Stop; retry next tick.
    Torn,
    /// Fewer than the 8 bytes needed for the frame header are available (EOF).
    Incomplete,
}

/// Corruption detected while framing — a non-zero, non-magic marker, or a size
/// outside the sanity bounds. This is an alarm condition, distinct from the
/// benign `Padding`/`Torn`/`Incomplete` "no data yet" outcomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordError {
    BadMagic([u8; 4]),
    BadSize(u32),
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordError::BadMagic(m) => write!(f, "non-magic, non-zero marker {m:02x?}"),
            RecordError::BadSize(s) => {
                write!(f, "record size {s} out of bounds [{MIN_BLK},{MAX_BLK}]")
            }
        }
    }
}
impl std::error::Error for RecordError {}

/// Parse one block record from `window` (the bytes available starting at the
/// cursor; `abs_pos` is the cursor's absolute file offset, used only to compute
/// `next_offset`). Pure and allocation-light — the unit of the tail reader.
///
/// - all-zero magic            -> `Padding`  (write frontier; stop)
/// - non-zero non-magic magic  -> `Err(BadMagic)` (corruption alarm)
/// - size out of [MIN,MAX]     -> `Err(BadSize)`  (corruption alarm)
/// - body not fully present    -> `Torn`     (retry next tick)
/// - < 8 bytes available       -> `Incomplete`(EOF)
/// - complete                  -> `Block`
pub fn parse_record(
    window: &[u8],
    abs_pos: u64,
    magic: [u8; 4],
) -> Result<RecordRead, RecordError> {
    if window.len() < 8 {
        return Ok(RecordRead::Incomplete);
    }
    let m = [window[0], window[1], window[2], window[3]];
    if m == [0, 0, 0, 0] {
        return Ok(RecordRead::Padding);
    }
    if m != magic {
        return Err(RecordError::BadMagic(m));
    }
    let size = u32::from_le_bytes([window[4], window[5], window[6], window[7]]);
    if !(MIN_BLK..=MAX_BLK).contains(&size) {
        return Err(RecordError::BadSize(size));
    }
    let end = 8usize + size as usize;
    if window.len() < end {
        // Header present, body not (yet) fully flushed.
        return Ok(RecordRead::Torn);
    }
    Ok(RecordRead::Block {
        block: window[8..end].to_vec(),
        next_offset: abs_pos + end as u64,
    })
}

// ---- private storage layer (tail_blocks + tail_meta) ----------------------
//
// The tail writes ONLY these two private CFs — never any canonical CF. Within
// `tail_blocks` two disjoint single-byte-prefixed keyspaces live side by side:
//   - record: `b'b' || block_hash(32)`                       -> serialized record
//   - index : `b'i' || height_be(4) || state(1) || hash(32)` -> empty
// `tail_meta` holds the cursor/anchor at key `b"cursor"`.
//
// The record and its index entry are always written in ONE `WriteBatch` (with
// the cursor advance), so a crash leaves a consistent state and replay is a
// no-op. On a state/height transition the OLD index entry is deleted and the new
// one inserted in the same batch, so the index can never drift from the record.

pub const CF_TAIL_BLOCKS: &str = "tail_blocks";
pub const CF_TAIL_META: &str = "tail_meta";

const KEY_PREFIX_RECORD: u8 = b'b';
const KEY_PREFIX_INDEX: u8 = b'i';
const CURSOR_KEY: &[u8] = b"cursor";
const REC_VERSION: u8 = 1;
const CURSOR_VERSION: u8 = 1;

/// Cache state of a tail-discovered block. NON-authoritative: the canonical/orphan
/// answer is re-derived at read time (§5.1E); this is a refreshable hint + drives
/// reconcile bookkeeping and metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TailState {
    Pending = 0,
    Canonical = 1,
    Orphan = 2,
    Unresolved = 3,
}

impl TailState {
    fn to_u8(self) -> u8 {
        self as u8
    }
    fn from_u8(b: u8) -> Option<Self> {
        match b {
            0 => Some(TailState::Pending),
            1 => Some(TailState::Canonical),
            2 => Some(TailState::Orphan),
            3 => Some(TailState::Unresolved),
            _ => None,
        }
    }
}

/// Durable tail reader position + anchor (in `tail_meta`). The anchor (hash of the
/// record ending at `offset`) and `file_len_watermark` let each tick detect a
/// `-reindex`/truncate before reading forward.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailCursor {
    pub file_no: u32,
    pub offset: u64,
    pub trailing_hash: [u8; 32],
    pub cumulative_blocks: u64,
    pub file_len_watermark: u64,
}

impl TailCursor {
    fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(61);
        v.push(CURSOR_VERSION);
        v.extend_from_slice(&self.file_no.to_le_bytes());
        v.extend_from_slice(&self.offset.to_le_bytes());
        v.extend_from_slice(&self.trailing_hash);
        v.extend_from_slice(&self.cumulative_blocks.to_le_bytes());
        v.extend_from_slice(&self.file_len_watermark.to_le_bytes());
        v
    }

    fn from_bytes(b: &[u8]) -> TailResult<Self> {
        if b.len() != 61 || b[0] != CURSOR_VERSION {
            return Err(format!(
                "bad tail cursor blob (len {}, ver {})",
                b.len(),
                b.first().copied().unwrap_or(0)
            )
            .into());
        }
        let file_no = u32::from_le_bytes(b[1..5].try_into().unwrap());
        let offset = u64::from_le_bytes(b[5..13].try_into().unwrap());
        let mut trailing_hash = [0u8; 32];
        trailing_hash.copy_from_slice(&b[13..45]);
        let cumulative_blocks = u64::from_le_bytes(b[45..53].try_into().unwrap());
        let file_len_watermark = u64::from_le_bytes(b[53..61].try_into().unwrap());
        Ok(TailCursor {
            file_no,
            offset,
            trailing_hash,
            cumulative_blocks,
            file_len_watermark,
        })
    }
}

/// A tail-discovered block record (value of `b'b' || block_hash`). `block_hash` is
/// the key and is not stored in the value. Hashes are internal byte order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailBlockRecord {
    pub block_hash: [u8; 32],
    pub prev_hash: [u8; 32],
    pub claimed_height: i32,
    pub header_work: [u8; 32],     // this block's work (big-endian)
    pub cumulative_work: [u8; 32], // best-effort cumulative work (big-endian)
    pub file_no: u32,
    pub offset: u64,
    pub state: TailState,
    pub first_seen: u64,      // unix secs (age clock = now - first_seen)
    pub last_reconciled: u64, // unix secs
    pub txids: Vec<[u8; 32]>, // internal byte order
}

impl TailBlockRecord {
    fn to_value(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(
            1 + 32 + 4 + 32 + 32 + 4 + 8 + 1 + 8 + 8 + 4 + self.txids.len() * 32,
        );
        v.push(REC_VERSION);
        v.extend_from_slice(&self.prev_hash);
        v.extend_from_slice(&self.claimed_height.to_le_bytes());
        v.extend_from_slice(&self.header_work);
        v.extend_from_slice(&self.cumulative_work);
        v.extend_from_slice(&self.file_no.to_le_bytes());
        v.extend_from_slice(&self.offset.to_le_bytes());
        v.push(self.state.to_u8());
        v.extend_from_slice(&self.first_seen.to_le_bytes());
        v.extend_from_slice(&self.last_reconciled.to_le_bytes());
        v.extend_from_slice(&(self.txids.len() as u32).to_le_bytes());
        for t in &self.txids {
            v.extend_from_slice(t);
        }
        v
    }

    fn from_kv(block_hash: [u8; 32], b: &[u8]) -> TailResult<Self> {
        const FIXED: usize = 1 + 32 + 4 + 32 + 32 + 4 + 8 + 1 + 8 + 8 + 4;
        if b.len() < FIXED || b[0] != REC_VERSION {
            return Err(format!("bad tail record blob (len {})", b.len()).into());
        }
        let mut p = 1usize;
        let mut prev_hash = [0u8; 32];
        prev_hash.copy_from_slice(&b[p..p + 32]);
        p += 32;
        let claimed_height = i32::from_le_bytes(b[p..p + 4].try_into().unwrap());
        p += 4;
        let mut header_work = [0u8; 32];
        header_work.copy_from_slice(&b[p..p + 32]);
        p += 32;
        let mut cumulative_work = [0u8; 32];
        cumulative_work.copy_from_slice(&b[p..p + 32]);
        p += 32;
        let file_no = u32::from_le_bytes(b[p..p + 4].try_into().unwrap());
        p += 4;
        let offset = u64::from_le_bytes(b[p..p + 8].try_into().unwrap());
        p += 8;
        let state = TailState::from_u8(b[p]).ok_or("bad tail state byte")?;
        p += 1;
        let first_seen = u64::from_le_bytes(b[p..p + 8].try_into().unwrap());
        p += 8;
        let last_reconciled = u64::from_le_bytes(b[p..p + 8].try_into().unwrap());
        p += 8;
        let count = u32::from_le_bytes(b[p..p + 4].try_into().unwrap()) as usize;
        p += 4;
        if b.len() != FIXED + count * 32 {
            return Err(format!(
                "tail record txid length mismatch (count {count}, len {})",
                b.len()
            )
            .into());
        }
        let mut txids = Vec::with_capacity(count);
        for _ in 0..count {
            let mut t = [0u8; 32];
            t.copy_from_slice(&b[p..p + 32]);
            p += 32;
            txids.push(t);
        }
        Ok(TailBlockRecord {
            block_hash,
            prev_hash,
            claimed_height,
            header_work,
            cumulative_work,
            file_no,
            offset,
            state,
            first_seen,
            last_reconciled,
            txids,
        })
    }
}

fn record_key(hash: &[u8; 32]) -> Vec<u8> {
    let mut k = Vec::with_capacity(33);
    k.push(KEY_PREFIX_RECORD);
    k.extend_from_slice(hash);
    k
}

/// Order-preserving big-endian encoding of an i32 height so a RocksDB range scan
/// visits records in ascending height order (heights are `to_le_bytes` everywhere
/// else; LE does not range-sort). Flipping the sign bit maps i32 order onto u32 BE.
fn height_be(h: i32) -> [u8; 4] {
    ((h as u32) ^ 0x8000_0000).to_be_bytes()
}

fn index_key(height: i32, state: TailState, hash: &[u8; 32]) -> Vec<u8> {
    let mut k = Vec::with_capacity(38);
    k.push(KEY_PREFIX_INDEX);
    k.extend_from_slice(&height_be(height));
    k.push(state.to_u8());
    k.extend_from_slice(hash);
    k
}

/// Eviction predicate (decidable, no future reference): a record may be evicted
/// only once it is `K` below the tip AND in a settled state. Pending/Unresolved
/// are never evicted.
pub fn is_evictable(claimed_height: i32, state: TailState, tip: i32, k: i32) -> bool {
    claimed_height < tip - k && matches!(state, TailState::Canonical | TailState::Orphan)
}

/// Handle onto the two private tail CFs. All mutations go through a `WriteBatch`
/// staged here and committed atomically by `commit`.
pub struct TailStore {
    db: Arc<DB>,
}

impl TailStore {
    pub fn new(db: Arc<DB>) -> Self {
        TailStore { db }
    }

    pub fn load_cursor(&self) -> TailResult<Option<TailCursor>> {
        let cf = self
            .db
            .cf_handle(CF_TAIL_META)
            .ok_or("tail_meta CF not found")?;
        match self.db.get_cf(&cf, CURSOR_KEY)? {
            Some(b) => Ok(Some(TailCursor::from_bytes(&b)?)),
            None => Ok(None),
        }
    }

    pub fn get_record(&self, hash: &[u8; 32]) -> TailResult<Option<TailBlockRecord>> {
        let cf = self
            .db
            .cf_handle(CF_TAIL_BLOCKS)
            .ok_or("tail_blocks CF not found")?;
        match self.db.get_cf(&cf, record_key(hash))? {
            Some(b) => Ok(Some(TailBlockRecord::from_kv(*hash, &b)?)),
            None => Ok(None),
        }
    }

    /// Stage a record upsert + its index entry into `batch`. If `prior` is given
    /// and its (height,state) index slot differs, the old index entry is deleted
    /// in the SAME batch so the index never drifts.
    pub fn stage_block(
        &self,
        batch: &mut WriteBatch,
        rec: &TailBlockRecord,
        prior: Option<&TailBlockRecord>,
    ) -> TailResult<()> {
        let cf = self
            .db
            .cf_handle(CF_TAIL_BLOCKS)
            .ok_or("tail_blocks CF not found")?;
        if let Some(p) = prior {
            if (p.claimed_height, p.state) != (rec.claimed_height, rec.state) {
                batch.delete_cf(&cf, index_key(p.claimed_height, p.state, &p.block_hash));
            }
        }
        batch.put_cf(&cf, record_key(&rec.block_hash), rec.to_value());
        batch.put_cf(
            &cf,
            index_key(rec.claimed_height, rec.state, &rec.block_hash),
            [0u8; 0],
        );
        Ok(())
    }

    /// Stage removal of a record + its index entry (eviction).
    pub fn stage_evict(&self, batch: &mut WriteBatch, rec: &TailBlockRecord) -> TailResult<()> {
        let cf = self
            .db
            .cf_handle(CF_TAIL_BLOCKS)
            .ok_or("tail_blocks CF not found")?;
        batch.delete_cf(&cf, record_key(&rec.block_hash));
        batch.delete_cf(
            &cf,
            index_key(rec.claimed_height, rec.state, &rec.block_hash),
        );
        Ok(())
    }

    /// Stage the cursor/anchor advance (committed in the same batch as the block).
    pub fn stage_cursor(&self, batch: &mut WriteBatch, cursor: &TailCursor) -> TailResult<()> {
        let cf = self
            .db
            .cf_handle(CF_TAIL_META)
            .ok_or("tail_meta CF not found")?;
        batch.put_cf(&cf, CURSOR_KEY, cursor.to_bytes());
        Ok(())
    }

    pub fn commit(&self, batch: WriteBatch) -> TailResult<()> {
        self.db.write(batch)?;
        Ok(())
    }

    /// Block hashes whose `claimed_height >= from_height`, ascending — the bounded
    /// `<K` reconcile/demotion scan (drives state re-derivation). Reads the BE
    /// index only; does not load records.
    pub fn hashes_from_height(&self, from_height: i32) -> TailResult<Vec<[u8; 32]>> {
        let cf = self
            .db
            .cf_handle(CF_TAIL_BLOCKS)
            .ok_or("tail_blocks CF not found")?;
        let mut start = Vec::with_capacity(5);
        start.push(KEY_PREFIX_INDEX);
        start.extend_from_slice(&height_be(from_height));
        let mut out = Vec::new();
        let iter = self
            .db
            .iterator_cf(&cf, IteratorMode::From(&start, Direction::Forward));
        for item in iter {
            let (key, _) = item?;
            if key.first() != Some(&KEY_PREFIX_INDEX) {
                break; // left the index keyspace
            }
            if key.len() != 38 {
                continue; // defensive: skip malformed index keys
            }
            let mut h = [0u8; 32];
            h.copy_from_slice(&key[6..38]);
            out.push(h);
        }
        Ok(out)
    }

    /// Block hashes in the `CLAIMED_UNKNOWN` bucket (parent never seen). These sort
    /// below every real height, so the `tip - K` reconcile scan misses them — they
    /// need their own retry/age pass to avoid an unbounded leak.
    pub fn hashes_at_unknown(&self) -> TailResult<Vec<[u8; 32]>> {
        let cf = self
            .db
            .cf_handle(CF_TAIL_BLOCKS)
            .ok_or("tail_blocks CF not found")?;
        let mut prefix = Vec::with_capacity(5);
        prefix.push(KEY_PREFIX_INDEX);
        prefix.extend_from_slice(&height_be(CLAIMED_UNKNOWN));
        let iter = self
            .db
            .iterator_cf(&cf, IteratorMode::From(&prefix, Direction::Forward));
        let mut out = Vec::new();
        for item in iter {
            let (key, _) = item?;
            if key.len() != 38 || key[..5] != prefix[..] {
                break; // left the CLAIMED_UNKNOWN prefix
            }
            let mut h = [0u8; 32];
            h.copy_from_slice(&key[6..38]);
            out.push(h);
        }
        Ok(out)
    }
}

// ---- 3c: header parsing + forward-map classifier --------------------------

/// Parsed block-header fields needed for classification. Hashes are internal
/// (little-endian) byte order, matching how the canonical maps store/key them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderInfo {
    pub block_hash: [u8; 32],
    pub prev_hash: [u8; 32],
    pub n_bits: u32,
    pub version: u32,
}

/// PIVX header size by block version (mirrors `offset_indexer::get_header_size`):
/// v1–3 = 80 bytes; v4+ = 112 (adds `hashFinalSaplingRoot`). The block hash is
/// SHA256d over the version-sized header.
fn header_size_for(version: u32) -> usize {
    match version {
        1..=3 => 80,
        4.. => 112,
        _ => 80, // version 0 / unexpected -> 80, matching offset_indexer::get_header_size
    }
}

/// Parse the header fields of a serialized block. `block` is the bytes WITHOUT the
/// magic+size frame (as returned by [`parse_record`]).
pub fn parse_header(block: &[u8]) -> TailResult<HeaderInfo> {
    use sha2::{Digest, Sha256};
    if block.len() < 80 {
        return Err(format!("block too short for a header ({} bytes)", block.len()).into());
    }
    let version = u32::from_le_bytes([block[0], block[1], block[2], block[3]]);
    let hsize = header_size_for(version);
    if block.len() < hsize {
        return Err(format!(
            "block {} bytes < header size {hsize} for v{version}",
            block.len()
        )
        .into());
    }
    // Block hash = SHA256d over the version-sized header (internal byte order).
    let first = Sha256::digest(&block[..hsize]);
    let second = Sha256::digest(&first);
    let mut block_hash = [0u8; 32];
    block_hash.copy_from_slice(&second);
    let mut prev_hash = [0u8; 32];
    prev_hash.copy_from_slice(&block[4..36]);
    let n_bits = u32::from_le_bytes([block[72], block[73], block[74], block[75]]);
    Ok(HeaderInfo {
        block_hash,
        prev_hash,
        n_bits,
        version,
    })
}

/// Sentinel `claimed_height` for a record whose height can't yet be derived
/// (parent not seen — rare, only across a crash gap given parents precede
/// children on disk). Sorts below every real height in the index, so the
/// `tip - K` reconcile scan misses it; the dedicated unknown-bucket pass in
/// `reconcile_pass` (via [`TailStore::hashes_at_unknown`]) retries parent
/// resolution and ages stale rows to `Unresolved` for prune to reclaim.
pub const CLAIMED_UNKNOWN: i32 = i32::MIN;

/// Best-effort big-endian 256-bit add for INFORMATIONAL cumulative work only
/// (never drives a canonical decision). Chainwork never approaches 2^256, so the
/// final carry-out is unreachable.
fn add_work_be(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut carry = 0u16;
    for i in (0..32).rev() {
        let s = a[i] as u16 + b[i] as u16 + carry;
        out[i] = (s & 0xff) as u8;
        carry = s >> 8;
    }
    out
}

impl TailStore {
    /// Canonical hash at `height` from the FORWARD map (`chain_metadata[height]`),
    /// returned in internal byte order. The forward map is the authoritative oracle
    /// (correctly deleted by `reorg.rs` on a reorg). Returns None if the height is
    /// not on the canonical chain (e.g. tip not there yet).
    pub fn canonical_hash_at(&self, height: i32) -> TailResult<Option<[u8; 32]>> {
        let cf = self
            .db
            .cf_handle("chain_metadata")
            .ok_or("chain_metadata CF not found")?;
        match self.db.get_cf(&cf, height.to_le_bytes())? {
            Some(v) if v.len() == 32 => {
                let mut internal = [0u8; 32];
                internal.copy_from_slice(&v);
                internal.reverse(); // stored display order -> internal
                Ok(Some(internal))
            }
            _ => Ok(None),
        }
    }

    /// Canonical height of `hash` (internal order), or None if not on the canonical
    /// chain. Uses the reverse `'h'` map (BOTH writer byte orderings) only as a
    /// HINT, then CONFIRMS against the forward map — so a leaked/stale `'h'` entry
    /// can never yield a false-canonical result.
    pub fn canonical_height_of(&self, hash_internal: &[u8; 32]) -> TailResult<Option<i32>> {
        let cf = self
            .db
            .cf_handle("chain_metadata")
            .ok_or("chain_metadata CF not found")?;
        let mut display = *hash_internal;
        display.reverse();
        for key_hash in [hash_internal, &display] {
            let mut hk = Vec::with_capacity(33);
            hk.push(b'h');
            hk.extend_from_slice(key_hash);
            if let Some(hbytes) = self.db.get_cf(&cf, &hk)? {
                if hbytes.len() == 4 {
                    let h = i32::from_le_bytes([hbytes[0], hbytes[1], hbytes[2], hbytes[3]]);
                    // Confirm against the authoritative forward map.
                    if let Some(fwd) = self.canonical_hash_at(h)? {
                        if &fwd == hash_internal {
                            return Ok(Some(h));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    /// Cumulative canonical chainwork at `height` (`'w'+height`), if present. May be
    /// absent (only written on the live path; skipped by a bulk reindex) — callers
    /// treat it as a best-effort optimization for informational cumulative work.
    fn canonical_chainwork_at(&self, height: i32) -> TailResult<Option<[u8; 32]>> {
        let cf = self
            .db
            .cf_handle("chain_metadata")
            .ok_or("chain_metadata CF not found")?;
        let mut key = Vec::with_capacity(5);
        key.push(b'w');
        key.extend_from_slice(&height.to_le_bytes());
        match self.db.get_cf(&cf, &key)? {
            Some(v) if v.len() == 32 => {
                let mut w = [0u8; 32];
                w.copy_from_slice(&v);
                Ok(Some(w))
            }
            _ => Ok(None),
        }
    }

    /// Resolve `prev_hash` to (this block's claimed_height, parent cumulative work).
    /// Parents precede children on disk (Q3), so the parent is either canonical or
    /// an already-recorded private side-branch block.
    fn resolve_parent(&self, prev_hash: &[u8; 32]) -> TailResult<(Option<i32>, Option<[u8; 32]>)> {
        // Parent canonical?
        if let Some(ph) = self.canonical_height_of(prev_hash)? {
            return Ok((Some(ph + 1), self.canonical_chainwork_at(ph)?));
        }
        // Parent a private side-branch record with a resolved height?
        if let Some(prec) = self.get_record(prev_hash)? {
            if prec.claimed_height >= 0 && prec.state != TailState::Unresolved {
                return Ok((Some(prec.claimed_height + 1), Some(prec.cumulative_work)));
            }
        }
        Ok((None, None)) // parent unknown / unresolved
    }

    /// Classify a freshly-read block: derive its height from the parent, then read
    /// the FORWARD map to decide canonical vs orphan vs pending. Pure reads of
    /// canonical state; returns the private record to persist (state is a cache
    /// hint — read-time §5.1E re-derivation is authoritative). `txids` are deferred
    /// (analytics-only; empty for now).
    pub fn classify(
        &self,
        block: &[u8],
        file_no: u32,
        offset: u64,
        now: u64,
    ) -> TailResult<TailBlockRecord> {
        let hdr = parse_header(block)?;
        let header_work = calculate_work_from_bits(hdr.n_bits);
        let (claimed_opt, parent_cum) = self.resolve_parent(&hdr.prev_hash)?;
        let cumulative_work = match parent_cum {
            Some(p) => add_work_be(&p, &header_work),
            None => header_work,
        };
        let state = match claimed_opt {
            None => TailState::Pending, // parent not yet resolvable
            Some(h) => match self.canonical_hash_at(h)? {
                Some(canon) if canon == hdr.block_hash => TailState::Canonical,
                Some(_) => TailState::Orphan,
                None => TailState::Pending, // height not on the forward map yet (tip behind)
            },
        };
        let claimed_height = claimed_opt.unwrap_or(CLAIMED_UNKNOWN);
        Ok(TailBlockRecord {
            block_hash: hdr.block_hash,
            prev_hash: hdr.prev_hash,
            claimed_height,
            header_work,
            cumulative_work,
            file_no,
            offset,
            state,
            first_seen: now,
            last_reconciled: now,
            txids: Vec::new(),
        })
    }
}

use crate::chainwork::calculate_work_from_bits;

// ---- 3d: window processing, reconcile, snapshot read ----------------------

/// Why a window-processing pass stopped (none of these advance past the point).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// Hit the all-zero pre-allocation region (write frontier). Normal; retry.
    Padding,
    /// Frame header present but body not fully flushed. Normal; retry.
    Torn,
    /// Ran out of windowed bytes before a full frame header (EOF for now).
    Incomplete,
    /// Non-zero, non-magic marker or out-of-range size — ALARM, do not advance.
    Corruption(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowOutcome {
    pub next_offset: u64,
    pub blocks_ingested: u64,
    pub stop: StopReason,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcileStats {
    pub scanned: usize,
    pub demotions: usize,
    pub promotions: usize,
    pub unresolved: usize,
}

impl TailStore {
    /// Process every complete record available in `window` (the bytes read from
    /// `file_no` starting at `start_offset`; `file_len` is the file's current
    /// length, persisted as the cursor watermark). Each block is classified and
    /// staged together with the advanced cursor in ONE `WriteBatch`, so a crash
    /// mid-tick leaves a consistent state and replay re-reads the last record
    /// idempotently. Framing corruption stops the pass (alarm) WITHOUT advancing
    /// the cursor over it.
    pub fn process_window(
        &self,
        file_no: u32,
        start_offset: u64,
        start_cumulative: u64,
        file_len: u64,
        window: &[u8],
        magic: [u8; 4],
        now: u64,
    ) -> TailResult<WindowOutcome> {
        let mut abs = start_offset;
        let mut ingested = 0u64;
        loop {
            let rel = (abs - start_offset) as usize;
            let stop = match parse_record(&window[rel.min(window.len())..], abs, magic) {
                Ok(RecordRead::Block { block, next_offset }) => {
                    let rec = self.classify(&block, file_no, abs, now)?;
                    let prior = self.get_record(&rec.block_hash)?;
                    let cursor = TailCursor {
                        file_no,
                        offset: next_offset,
                        trailing_hash: rec.block_hash,
                        cumulative_blocks: start_cumulative + ingested + 1,
                        file_len_watermark: file_len,
                    };
                    let mut batch = WriteBatch::default();
                    self.stage_block(&mut batch, &rec, prior.as_ref())?;
                    self.stage_cursor(&mut batch, &cursor)?;
                    self.commit(batch)?;
                    ingested += 1;
                    abs = next_offset;
                    continue;
                }
                Ok(RecordRead::Padding) => StopReason::Padding,
                Ok(RecordRead::Torn) => StopReason::Torn,
                Ok(RecordRead::Incomplete) => StopReason::Incomplete,
                Err(e) => StopReason::Corruption(e.to_string()),
            };
            return Ok(WindowOutcome {
                next_offset: abs,
                blocks_ingested: ingested,
                stop,
            });
        }
    }

    /// Monitor-independent reconcile pass: re-derive the cached state of every
    /// record with `claimed_height >= tip - K` (bounded `<K` via the BE index) from
    /// the FORWARD map, handling canonical->orphan demotion and orphan->canonical
    /// promotion statelessly (crash-safe by recomputation). A pending record older
    /// than `MAX_PENDING_AGE_SECS` is finalized to the terminal `Unresolved`.
    pub fn reconcile_pass(&self, tip: i32, now: u64) -> TailResult<ReconcileStats> {
        let from = tip.saturating_sub(K_RETENTION);
        let hashes = self.hashes_from_height(from)?;
        let mut stats = ReconcileStats {
            scanned: hashes.len(),
            ..Default::default()
        };
        for h in hashes {
            let Some(rec) = self.get_record(&h)? else {
                continue;
            };
            let new_state = match self.canonical_hash_at(rec.claimed_height)? {
                Some(c) if c == rec.block_hash => TailState::Canonical,
                Some(_) => TailState::Orphan,
                None => {
                    // Unresolved if it was already unresolved OR it has aged past the
                    // pending window; otherwise still Pending. (Two identical Unresolved
                    // arms collapsed into one condition.)
                    if rec.state == TailState::Unresolved
                        || now.saturating_sub(rec.first_seen) > MAX_PENDING_AGE_SECS
                    {
                        TailState::Unresolved
                    } else {
                        TailState::Pending
                    }
                }
            };
            if new_state != rec.state {
                match (rec.state, new_state) {
                    (TailState::Canonical, TailState::Orphan) => stats.demotions += 1,
                    (TailState::Orphan, TailState::Canonical) => stats.promotions += 1,
                    (_, TailState::Unresolved) => stats.unresolved += 1,
                    _ => {}
                }
                let mut nr = rec.clone();
                nr.state = new_state;
                nr.last_reconciled = now;
                let mut batch = WriteBatch::default();
                self.stage_block(&mut batch, &nr, Some(&rec))?;
                self.commit(batch)?;
            }
        }

        // Retry the parent-unknown bucket separately (it sorts below the scan): a
        // parent may have since been recorded (resolve it to a real height), else
        // age it to terminal Unresolved so prune can reclaim it (bounds growth).
        for h in self.hashes_at_unknown()? {
            let Some(rec) = self.get_record(&h)? else {
                continue;
            };
            stats.scanned += 1;
            if rec.state == TailState::Unresolved {
                continue; // already terminal; prune handles reclamation
            }
            let (claimed_opt, parent_cum) = self.resolve_parent(&rec.prev_hash)?;
            let mut nr = rec.clone();
            let changed = match claimed_opt {
                Some(height) => {
                    nr.claimed_height = height;
                    if let Some(p) = parent_cum {
                        nr.cumulative_work = add_work_be(&p, &rec.header_work);
                    }
                    nr.state = match self.canonical_hash_at(height)? {
                        Some(c) if c == rec.block_hash => TailState::Canonical,
                        Some(_) => TailState::Orphan,
                        None => TailState::Pending,
                    };
                    true
                }
                None if now.saturating_sub(rec.first_seen) > MAX_PENDING_AGE_SECS => {
                    nr.state = TailState::Unresolved;
                    stats.unresolved += 1;
                    true
                }
                None => false,
            };
            if changed {
                nr.last_reconciled = now;
                let mut batch = WriteBatch::default();
                self.stage_block(&mut batch, &nr, Some(&rec))?;
                self.commit(batch)?;
            }
        }
        Ok(stats)
    }

    /// §5.1E authoritative read: under ONE consistent snapshot spanning both the
    /// tail records and `chain_metadata`, re-derive canonical-vs-orphan for every
    /// record and return the orphan set. The stored `state` cache is NOT consulted,
    /// so a concurrent reorg cannot tear the join.
    pub fn orphans_under_snapshot(&self) -> TailResult<Vec<TailBlockRecord>> {
        let cf_tb = self
            .db
            .cf_handle(CF_TAIL_BLOCKS)
            .ok_or("tail_blocks CF not found")?;
        let cf_cm = self
            .db
            .cf_handle("chain_metadata")
            .ok_or("chain_metadata CF not found")?;
        let snap = self.db.snapshot();
        let mut ro_iter = ReadOptions::default();
        ro_iter.set_snapshot(&snap);
        let mut ro_get = ReadOptions::default();
        ro_get.set_snapshot(&snap);

        let start = [KEY_PREFIX_RECORD];
        let iter = self.db.iterator_cf_opt(
            &cf_tb,
            ro_iter,
            IteratorMode::From(&start, Direction::Forward),
        );
        let mut out = Vec::new();
        for item in iter {
            let (key, val) = item?;
            if key.first() != Some(&KEY_PREFIX_RECORD) {
                break; // records ('b') sort before the index ('i'); we're past them
            }
            if key.len() != 33 {
                continue;
            }
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&key[1..33]);
            let rec = TailBlockRecord::from_kv(hash, &val)?;
            // authoritative re-derive vs the forward map UNDER THE SAME snapshot
            let canon =
                match self
                    .db
                    .get_cf_opt(&cf_cm, rec.claimed_height.to_le_bytes(), &ro_get)?
                {
                    Some(v) if v.len() == 32 => {
                        let mut internal = [0u8; 32];
                        internal.copy_from_slice(&v);
                        internal.reverse();
                        Some(internal)
                    }
                    _ => None,
                };
            if matches!(canon, Some(c) if c != rec.block_hash) {
                out.push(rec);
            }
        }
        Ok(out)
    }

    /// Evict settled records (`canonical`/`orphan`) more than `K` below `tip`,
    /// bounded to `max_evict` per call. Pending/Unresolved are never evicted. The
    /// BE index is height-ordered, so we scan from the bottom and stop once we
    /// reach within `K` of the tip. Returns the number evicted.
    pub fn prune_pass(&self, tip: i32, max_evict: usize) -> TailResult<usize> {
        let cf = self
            .db
            .cf_handle(CF_TAIL_BLOCKS)
            .ok_or("tail_blocks CF not found")?;
        let cutoff = tip.saturating_sub(K_RETENTION);
        let start = [KEY_PREFIX_INDEX]; // lowest index key
        let iter = self
            .db
            .iterator_cf(&cf, IteratorMode::From(&start, Direction::Forward));
        let mut victims: Vec<[u8; 32]> = Vec::new();
        for item in iter {
            let (key, _) = item?;
            if key.first() != Some(&KEY_PREFIX_INDEX) || key.len() != 38 {
                break;
            }
            let height = decode_height_be([key[1], key[2], key[3], key[4]]);
            if height >= cutoff {
                break; // index is height-ordered; nothing below the cutoff remains
            }
            // Evict settled records (canonical/orphan) AND terminal Unresolved ones
            // (e.g. aged-out CLAIMED_UNKNOWN parent-never-seen rows) so growth stays
            // bounded. Pending is never evicted (it may still resolve).
            let reclaimable = matches!(
                TailState::from_u8(key[5]),
                Some(TailState::Canonical) | Some(TailState::Orphan) | Some(TailState::Unresolved)
            );
            if reclaimable {
                let mut h = [0u8; 32];
                h.copy_from_slice(&key[6..38]);
                victims.push(h);
                if victims.len() >= max_evict {
                    break;
                }
            }
        }
        let mut batch = WriteBatch::default();
        let mut evicted = 0;
        for h in victims {
            if let Some(rec) = self.get_record(&h)? {
                self.stage_evict(&mut batch, &rec)?;
                evicted += 1;
            }
        }
        if evicted > 0 {
            self.commit(batch)?;
        }
        Ok(evicted)
    }

    /// Rough current key count in `tail_blocks` (records + index ≈ 2 per block),
    /// used only for the `MAX_TAIL_BLOCKS` safety pause.
    pub fn estimate_keys(&self) -> u64 {
        self.db
            .cf_handle(CF_TAIL_BLOCKS)
            .and_then(|cf| {
                self.db
                    .property_int_value_cf(&cf, "rocksdb.estimate-num-keys")
                    .ok()
                    .flatten()
            })
            .unwrap_or(0)
    }
}

/// Inverse of [`height_be`].
fn decode_height_be(b: [u8; 4]) -> i32 {
    (u32::from_be_bytes(b) ^ 0x8000_0000) as i32
}

// ---- 3d: async runtime + flag-gated spawn ---------------------------------

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn blk_path(blk_dir: &Path, file_no: u32) -> PathBuf {
    blk_dir.join(format!("blk{file_no:05}.dat"))
}

/// Highest `blkNNNNN.dat` number present (where a no-cursor tail starts).
async fn highest_blk_file(blk_dir: &Path) -> Option<u32> {
    let mut rd = tokio::fs::read_dir(blk_dir).await.ok()?;
    let mut max: Option<u32> = None;
    while let Ok(Some(e)) = rd.next_entry().await {
        if let Some(name) = e.file_name().to_str() {
            if let Some(num) = name
                .strip_prefix("blk")
                .and_then(|s| s.strip_suffix(".dat"))
            {
                if let Ok(n) = num.parse::<u32>() {
                    max = Some(max.map_or(n, |m| m.max(n)));
                }
            }
        }
    }
    max
}

async fn read_window(path: &Path, offset: u64, len: u64) -> TailResult<Vec<u8>> {
    use tokio::io::SeekFrom;
    let mut f = tokio::fs::File::open(path).await?;
    f.seek(SeekFrom::Start(offset)).await?;
    let mut buf = vec![0u8; len as usize];
    let mut read = 0usize;
    while read < buf.len() {
        let n = f.read(&mut buf[read..]).await?;
        if n == 0 {
            break;
        }
        read += n;
    }
    buf.truncate(read);
    Ok(buf)
}

fn roll_to_next_file(store: &TailStore, from: &TailCursor) -> TailResult<()> {
    let nc = TailCursor {
        file_no: from.file_no + 1,
        offset: 0,
        trailing_hash: from.trailing_hash,
        cumulative_blocks: from.cumulative_blocks,
        file_len_watermark: 0,
    };
    let mut batch = WriteBatch::default();
    store.stage_cursor(&mut batch, &nc)?;
    store.commit(batch)
}

/// One read tick: advance the cursor through complete records now on disk, with
/// the reindex/shrink guard and file rollover. Returns blocks ingested.
async fn tail_read_tick(
    store: &TailStore,
    blk_dir: &Path,
    magic: [u8; 4],
    cap: u64,
) -> TailResult<u64> {
    let cursor = match store.load_cursor()? {
        Some(c) => c,
        None => match highest_blk_file(blk_dir).await {
            Some(f) => TailCursor {
                file_no: f,
                offset: 0,
                trailing_hash: [0u8; 32],
                cumulative_blocks: 0,
                file_len_watermark: 0,
            },
            None => return Ok(0), // no blk files yet
        },
    };

    let path = blk_path(blk_dir, cursor.file_no);
    let len = match tokio::fs::metadata(&path).await {
        Ok(m) => m.len(),
        Err(_) => {
            warn!(file = %path.display(), "blk_tail: cursor file missing (pruned?) — halting tick");
            return Ok(0);
        }
    };

    // Reindex/truncate guard: a blk file never shrinks in normal operation.
    if len < cursor.file_len_watermark {
        warn!(file = %path.display(), len, watermark = cursor.file_len_watermark,
            "blk_tail: cursor file shrank — possible -reindex; halting (no advance)");
        return Ok(0);
    }

    let mut ingested = 0u64;
    // A file is "exhausted" (safe to roll past) only when there was no new data,
    // OR we read to EOF and stopped on Padding/Incomplete (trailing zero padding /
    // no full frame). A `Torn` stop means a real record is still being flushed —
    // we must NOT roll past it or that block is permanently SKIPPED. `Corruption`
    // halts in place.
    let mut file_exhausted = len <= cursor.offset;
    if len > cursor.offset {
        let remaining = len - cursor.offset;
        let to_read = remaining.min(cap);
        let drained_to_eof = to_read == remaining;
        let window = read_window(&path, cursor.offset, to_read).await?;
        let outcome = store.process_window(
            cursor.file_no,
            cursor.offset,
            cursor.cumulative_blocks,
            len,
            &window,
            magic,
            now_secs(),
        )?;
        ingested = outcome.blocks_ingested;
        match &outcome.stop {
            StopReason::Corruption(msg) => {
                warn!(file = %path.display(), offset = outcome.next_offset, msg = %msg,
                    "blk_tail: framing corruption — halting this file (no advance past it)");
                return Ok(ingested);
            }
            StopReason::Padding | StopReason::Incomplete => file_exhausted = drained_to_eof,
            StopReason::Torn => file_exhausted = false, // a real record is still flushing — wait
        }
    }

    // Rollover: Core opens blk{n+1} only once blk{n} is full. Roll only when this
    // file is genuinely exhausted (above) AND the next file exists.
    if file_exhausted
        && tokio::fs::try_exists(blk_path(blk_dir, cursor.file_no + 1))
            .await
            .unwrap_or(false)
    {
        let latest = store.load_cursor()?.unwrap_or(cursor);
        roll_to_next_file(store, &latest)?;
    }
    Ok(ingested)
}

/// The flag-gated tail task. Reads new blocks every ~2 s; runs the reconcile +
/// prune passes every `RECONCILE_INTERVAL_SECS`. Never writes any canonical CF.
pub async fn run_tail(db: Arc<DB>, blk_dir: PathBuf, magic: [u8; 4]) {
    const TICK_SECS: u64 = 2;
    const WINDOW_CAP: u64 = 8 * 1024 * 1024;
    const MAX_EVICT_PER_PASS: usize = 10_000;

    info!(blk_dir = %blk_dir.display(), "blk_tail: live orphan discovery started (opt-in)");
    let store = TailStore::new(db.clone());
    let mut ticker = tokio::time::interval(Duration::from_secs(TICK_SECS));
    let mut since_reconcile = 0u64;
    let mut last_prune_tip = i32::MIN;

    loop {
        ticker.tick().await;

        // Memory safety: pause ingest if eviction can't keep up; reconcile/prune
        // below still run, so it auto-resumes as the tip advances and drains.
        let keys = store.estimate_keys();
        if keys > MAX_TAIL_BLOCKS * 2 {
            warn!(
                keys,
                "blk_tail: MAX_TAIL_BLOCKS reached — pausing ingest this tick"
            );
        } else if let Err(e) = tail_read_tick(&store, &blk_dir, magic, WINDOW_CAP).await {
            warn!(error = %e, "blk_tail: read tick error");
        }

        since_reconcile += TICK_SECS;
        if since_reconcile >= RECONCILE_INTERVAL_SECS {
            since_reconcile = 0;
            let tip = crate::chain_state::get_sync_height(&db).unwrap_or(0);
            match store.reconcile_pass(tip, now_secs()) {
                Ok(s) if s.demotions + s.promotions + s.unresolved > 0 => info!(
                    scanned = s.scanned,
                    demotions = s.demotions,
                    promotions = s.promotions,
                    unresolved = s.unresolved,
                    "blk_tail: reconcile"
                ),
                Ok(_) => {}
                Err(e) => warn!(error = %e, "blk_tail: reconcile error"),
            }
            // Prune only when the tip advanced — freezes eviction while the monitor stalls.
            if tip > last_prune_tip {
                last_prune_tip = tip;
                match store.prune_pass(tip, MAX_EVICT_PER_PASS) {
                    Ok(n) if n > 0 => {
                        info!(evicted = n, "blk_tail: pruned settled records below tip-K")
                    }
                    Ok(_) => {}
                    Err(e) => warn!(error = %e, "blk_tail: prune error"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(magic: [u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(8 + body.len());
        v.extend_from_slice(&magic);
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn parses_a_complete_record_and_advances() {
        let body = vec![0xABu8; MIN_BLK as usize + 10];
        let buf = frame(PIVX_MAGIC, &body);
        let out = parse_record(&buf, 1000, PIVX_MAGIC).unwrap();
        match out {
            RecordRead::Block { block, next_offset } => {
                assert_eq!(block, body);
                assert_eq!(next_offset, 1000 + 8 + body.len() as u64);
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn all_zero_magic_is_padding_not_corruption() {
        let buf = vec![0u8; 64]; // pre-allocated zero region ahead of the frontier
        assert_eq!(
            parse_record(&buf, 0, PIVX_MAGIC).unwrap(),
            RecordRead::Padding
        );
    }

    #[test]
    fn non_zero_non_magic_is_corruption() {
        let mut buf = vec![0u8; 16];
        buf[0..4].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        match parse_record(&buf, 0, PIVX_MAGIC) {
            Err(RecordError::BadMagic(m)) => assert_eq!(m, [0xde, 0xad, 0xbe, 0xef]),
            other => panic!("expected BadMagic, got {other:?}"),
        }
    }

    #[test]
    fn torn_when_body_incomplete() {
        let body = vec![0x11u8; MIN_BLK as usize + 100];
        let mut buf = frame(PIVX_MAGIC, &body);
        buf.truncate(8 + 10); // header + only 10 of the body bytes flushed
        assert_eq!(parse_record(&buf, 0, PIVX_MAGIC).unwrap(), RecordRead::Torn);
    }

    #[test]
    fn incomplete_when_under_frame_header() {
        let buf = vec![0x90u8, 0xc4, 0xfd]; // 3 bytes, < 8
        assert_eq!(
            parse_record(&buf, 0, PIVX_MAGIC).unwrap(),
            RecordRead::Incomplete
        );
    }

    #[test]
    fn size_below_min_is_corruption() {
        let mut buf = vec![0u8; 8];
        buf[0..4].copy_from_slice(&PIVX_MAGIC);
        buf[4..8].copy_from_slice(&(10u32).to_le_bytes()); // < MIN_BLK
        match parse_record(&buf, 0, PIVX_MAGIC) {
            Err(RecordError::BadSize(s)) => assert_eq!(s, 10),
            other => panic!("expected BadSize, got {other:?}"),
        }
    }

    #[test]
    fn size_above_max_is_corruption() {
        let mut buf = vec![0u8; 8];
        buf[0..4].copy_from_slice(&PIVX_MAGIC);
        buf[4..8].copy_from_slice(&(MAX_BLK + 1).to_le_bytes());
        match parse_record(&buf, 0, PIVX_MAGIC) {
            Err(RecordError::BadSize(s)) => assert_eq!(s, MAX_BLK + 1),
            other => panic!("expected BadSize, got {other:?}"),
        }
    }

    #[test]
    fn in_body_magic_cannot_desync_framing() {
        // A coincidental magic sequence INSIDE a block body must not matter: we
        // frame strictly by size, so the next record starts exactly after `size`.
        let mut body = vec![0x11u8; MIN_BLK as usize + 20];
        body[5..9].copy_from_slice(&PIVX_MAGIC); // decoy magic mid-body
        let buf = frame(PIVX_MAGIC, &body);
        match parse_record(&buf, 0, PIVX_MAGIC).unwrap() {
            RecordRead::Block { block, next_offset } => {
                assert_eq!(block, body);
                assert_eq!(next_offset, 8 + body.len() as u64);
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn retention_constants_are_consistent() {
        // K must cover the deepest reorg/lag we claim to handle exactly, + K_confirm.
        assert!(K_RETENTION >= EXPECTED_REORG_DEPTH.max(MAX_MONITOR_LAG) + K_CONFIRM);
        // Compile-time invariant (constants), so it can't silently regress.
        const _: () = assert!(MAX_BLK > MIN_BLK);
    }

    // ---- 3b storage layer ----

    fn tail_test_store() -> (TailStore, tempfile::TempDir) {
        use rocksdb::{Options, DB};
        use tempfile::TempDir;
        let temp = TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = DB::open_cf(&opts, temp.path(), [CF_TAIL_BLOCKS, CF_TAIL_META]).unwrap();
        (TailStore::new(Arc::new(db)), temp)
    }

    fn rec(hash: u8, prev: u8, height: i32, state: TailState) -> TailBlockRecord {
        TailBlockRecord {
            block_hash: [hash; 32],
            prev_hash: [prev; 32],
            claimed_height: height,
            header_work: [7u8; 32],
            cumulative_work: [9u8; 32],
            file_no: 3,
            offset: 12345,
            state,
            first_seen: 1000,
            last_reconciled: 1000,
            txids: vec![[hash; 32], [hash.wrapping_add(1); 32]],
        }
    }

    fn commit_block(store: &TailStore, r: &TailBlockRecord, prior: Option<&TailBlockRecord>) {
        let mut batch = rocksdb::WriteBatch::default();
        store.stage_block(&mut batch, r, prior).unwrap();
        store.commit(batch).unwrap();
    }

    #[test]
    fn cursor_round_trips() {
        let c = TailCursor {
            file_no: 42,
            offset: 9_876_543_210,
            trailing_hash: [0xAB; 32],
            cumulative_blocks: 555,
            file_len_watermark: 134_217_728,
        };
        let bytes = c.to_bytes();
        assert_eq!(bytes.len(), 61);
        assert_eq!(TailCursor::from_bytes(&bytes).unwrap(), c);
    }

    #[test]
    fn record_round_trips_including_txids() {
        let r = rec(0x11, 0x10, 4242, TailState::Orphan);
        let val = r.to_value();
        let back = TailBlockRecord::from_kv(r.block_hash, &val).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn cursor_persists_and_loads() {
        let (store, _t) = tail_test_store();
        assert!(store.load_cursor().unwrap().is_none());
        let c = TailCursor {
            file_no: 1,
            offset: 64,
            trailing_hash: [3; 32],
            cumulative_blocks: 1,
            file_len_watermark: 64,
        };
        let mut batch = rocksdb::WriteBatch::default();
        store.stage_cursor(&mut batch, &c).unwrap();
        store.commit(batch).unwrap();
        assert_eq!(store.load_cursor().unwrap().unwrap(), c);
    }

    #[test]
    fn upsert_writes_record_and_one_index_entry() {
        let (store, _t) = tail_test_store();
        let r = rec(0x22, 0x21, 100, TailState::Pending);
        commit_block(&store, &r, None);
        assert_eq!(store.get_record(&r.block_hash).unwrap().unwrap(), r);
        // exactly one index entry, for our hash, visible from any height <= 100.
        let hashes = store.hashes_from_height(0).unwrap();
        assert_eq!(hashes, vec![[0x22; 32]]);
    }

    #[test]
    fn state_transition_swaps_index_entry_in_one_batch() {
        let (store, _t) = tail_test_store();
        let p = rec(0x33, 0x32, 200, TailState::Pending);
        commit_block(&store, &p, None);
        // Promote/demote: same hash + height, new state.
        let o = rec(0x33, 0x32, 200, TailState::Orphan);
        commit_block(&store, &o, Some(&p));
        // The record reflects the new state, and there is STILL exactly one index
        // entry (the old (height,Pending) slot was deleted in the same batch).
        assert_eq!(
            store.get_record(&o.block_hash).unwrap().unwrap().state,
            TailState::Orphan
        );
        assert_eq!(store.hashes_from_height(0).unwrap(), vec![[0x33; 32]]);
    }

    #[test]
    fn reupsert_is_idempotent() {
        let (store, _t) = tail_test_store();
        let r = rec(0x44, 0x43, 300, TailState::Canonical);
        commit_block(&store, &r, None);
        // Replay (crash recovery) — re-stage the identical record with prior == itself.
        commit_block(&store, &r, Some(&r));
        assert_eq!(store.get_record(&r.block_hash).unwrap().unwrap(), r);
        assert_eq!(store.hashes_from_height(0).unwrap(), vec![[0x44; 32]]);
    }

    #[test]
    fn scan_is_height_ordered_and_lower_bounded() {
        let (store, _t) = tail_test_store();
        for (h, height) in [(0x01u8, 10i32), (0x02, 30), (0x03, 20)] {
            commit_block(&store, &rec(h, h - 1, height, TailState::Orphan), None);
        }
        // Ascending by height: 10, 20, 30 -> hashes 0x01, 0x03, 0x02.
        assert_eq!(
            store.hashes_from_height(0).unwrap(),
            vec![[0x01; 32], [0x03; 32], [0x02; 32]]
        );
        // Lower bound excludes heights below tip-K.
        assert_eq!(store.hashes_from_height(25).unwrap(), vec![[0x02; 32]]);
    }

    #[test]
    fn eviction_removes_record_and_index() {
        let (store, _t) = tail_test_store();
        let r = rec(0x55, 0x54, 5, TailState::Orphan);
        commit_block(&store, &r, None);
        let mut batch = rocksdb::WriteBatch::default();
        store.stage_evict(&mut batch, &r).unwrap();
        store.commit(batch).unwrap();
        assert!(store.get_record(&r.block_hash).unwrap().is_none());
        assert!(store.hashes_from_height(0).unwrap().is_empty());
    }

    #[test]
    fn evictable_predicate() {
        // settled + K below tip -> evictable
        assert!(is_evictable(100, TailState::Canonical, 500, K_RETENTION));
        assert!(is_evictable(100, TailState::Orphan, 500, K_RETENTION));
        // within K of tip -> not evictable
        assert!(!is_evictable(400, TailState::Canonical, 500, K_RETENTION));
        // pending/unresolved are never evictable, regardless of depth
        assert!(!is_evictable(1, TailState::Pending, 100_000, K_RETENTION));
        assert!(!is_evictable(
            1,
            TailState::Unresolved,
            100_000,
            K_RETENTION
        ));
    }

    // ---- 3c classifier ----

    const NBITS: u32 = 0x1d00_ffff; // a valid compact target -> non-zero work

    fn store_with_chain() -> (TailStore, tempfile::TempDir) {
        use rocksdb::{Options, DB};
        use tempfile::TempDir;
        let temp = TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = DB::open_cf(
            &opts,
            temp.path(),
            [CF_TAIL_BLOCKS, CF_TAIL_META, "chain_metadata"],
        )
        .unwrap();
        (TailStore::new(Arc::new(db)), temp)
    }

    /// Seed a canonical block at `height`: forward `height -> display(hash)`, plus
    /// the reverse `'h'` entry in the requested byte order(s).
    fn seed_canonical(
        store: &TailStore,
        height: i32,
        hash_internal: [u8; 32],
        internal_h: bool,
        display_h: bool,
    ) {
        let cf = store.db.cf_handle("chain_metadata").unwrap();
        let mut display = hash_internal;
        display.reverse();
        store.db.put_cf(&cf, height.to_le_bytes(), display).unwrap();
        if internal_h {
            let mut hk = vec![b'h'];
            hk.extend_from_slice(&hash_internal);
            store.db.put_cf(&cf, hk, height.to_le_bytes()).unwrap();
        }
        if display_h {
            let mut hk = vec![b'h'];
            hk.extend_from_slice(&display);
            store.db.put_cf(&cf, hk, height.to_le_bytes()).unwrap();
        }
    }

    fn make_header(version: u32, prev_internal: [u8; 32], nbits: u32) -> Vec<u8> {
        let mut h = vec![0u8; 80];
        h[0..4].copy_from_slice(&version.to_le_bytes());
        h[4..36].copy_from_slice(&prev_internal);
        h[72..76].copy_from_slice(&nbits.to_le_bytes());
        h
    }

    fn hash_of(block: &[u8]) -> [u8; 32] {
        parse_header(block).unwrap().block_hash
    }

    #[test]
    fn classifies_canonical_block() {
        let (store, _t) = store_with_chain();
        let parent = [0xAA; 32];
        seed_canonical(&store, 10, parent, true, false);
        let block = make_header(1, parent, NBITS);
        let hb = hash_of(&block);
        seed_canonical(&store, 11, hb, false, false); // forward[11] == hb
        let r = store.classify(&block, 1, 100, 1000).unwrap();
        assert_eq!(r.claimed_height, 11);
        assert_eq!(r.state, TailState::Canonical);
        assert_eq!(r.block_hash, hb);
        assert_eq!(r.prev_hash, parent);
    }

    #[test]
    fn classifies_orphan_when_forward_hash_differs() {
        let (store, _t) = store_with_chain();
        let parent = [0xAA; 32];
        seed_canonical(&store, 10, parent, true, false);
        seed_canonical(&store, 11, [0xCC; 32], false, false); // a DIFFERENT block won height 11
        let block = make_header(1, parent, NBITS);
        let r = store.classify(&block, 1, 100, 1000).unwrap();
        assert_eq!(r.claimed_height, 11);
        assert_eq!(r.state, TailState::Orphan);
    }

    #[test]
    fn pending_when_height_not_on_forward_map_yet() {
        let (store, _t) = store_with_chain();
        let parent = [0xAA; 32];
        seed_canonical(&store, 10, parent, true, false); // tip is at 10; 11 not present
        let block = make_header(1, parent, NBITS);
        let r = store.classify(&block, 1, 100, 1000).unwrap();
        assert_eq!(r.claimed_height, 11); // height known...
        assert_eq!(r.state, TailState::Pending); // ...but canonical hash at 11 unknown yet
    }

    #[test]
    fn parent_resolves_via_display_order_h_key() {
        let (store, _t) = store_with_chain();
        let parent = [0xAB; 32];
        // monitor-style: only the DISPLAY-order 'h' key exists.
        seed_canonical(&store, 10, parent, false, true);
        let block = make_header(1, parent, NBITS);
        let hb = hash_of(&block);
        seed_canonical(&store, 11, hb, false, false);
        let r = store.classify(&block, 1, 100, 1000).unwrap();
        assert_eq!((r.claimed_height, r.state), (11, TailState::Canonical));
    }

    #[test]
    fn stale_h_entry_cannot_yield_false_canonical() {
        let (store, _t) = store_with_chain();
        let parent = [0xAA; 32];
        seed_canonical(&store, 10, parent, true, false);
        // A LEAKED reverse 'h' for a hash that is NOT the forward hash at 10.
        let stale = [0xEE; 32];
        let cf = store.db.cf_handle("chain_metadata").unwrap();
        let mut hk = vec![b'h'];
        hk.extend_from_slice(&stale);
        store.db.put_cf(&cf, hk, 10i32.to_le_bytes()).unwrap();
        // The forward-map confirm rejects the stale hint -> not canonical.
        assert!(store.canonical_height_of(&stale).unwrap().is_none());
        // A block built on `stale` (not actually canonical, no private record) is
        // Pending with an UNKNOWN height — never falsely Canonical/Orphan.
        let block = make_header(1, stale, NBITS);
        let r = store.classify(&block, 1, 100, 1000).unwrap();
        assert_eq!(r.state, TailState::Pending);
        assert_eq!(r.claimed_height, CLAIMED_UNKNOWN);
    }

    #[test]
    fn derives_height_from_private_side_branch_parent() {
        let (store, _t) = store_with_chain();
        // A private side-branch parent at height 20.
        let parent = [0xBE; 32];
        let p_rec = TailBlockRecord {
            block_hash: parent,
            prev_hash: [0xBD; 32],
            claimed_height: 20,
            header_work: [1u8; 32],
            cumulative_work: [5u8; 32],
            file_no: 1,
            offset: 0,
            state: TailState::Orphan,
            first_seen: 1,
            last_reconciled: 1,
            txids: vec![],
        };
        commit_block(&store, &p_rec, None);
        // Child builds on the private parent; forward[21] absent -> Pending, but the
        // height is derived as parent.claimed_height + 1 = 21.
        let block = make_header(1, parent, NBITS);
        let r = store.classify(&block, 1, 100, 1000).unwrap();
        assert_eq!(r.claimed_height, 21);
        assert_eq!(r.state, TailState::Pending);
        // cumulative_work = parent cumulative + this header's work (informational).
        assert_ne!(r.cumulative_work, r.header_work); // parent's [5;32] was added in
    }

    // ---- 3d window processing / reconcile / snapshot read ----

    fn set_forward(store: &TailStore, height: i32, hash_internal: [u8; 32]) {
        let cf = store.db.cf_handle("chain_metadata").unwrap();
        let mut d = hash_internal;
        d.reverse();
        store.db.put_cf(&cf, height.to_le_bytes(), d).unwrap();
    }

    #[test]
    fn process_window_ingests_chain_and_advances_cursor() {
        let (store, _t) = store_with_chain();
        let parent = [0xAA; 32];
        seed_canonical(&store, 10, parent, true, false);
        let h1 = make_header(1, parent, NBITS);
        let hash1 = hash_of(&h1);
        let h2 = make_header(1, hash1, NBITS); // builds on h1
        let hash2 = hash_of(&h2);

        let mut window = frame(PIVX_MAGIC, &h1);
        window.extend_from_slice(&frame(PIVX_MAGIC, &h2));
        window.extend_from_slice(&[0u8; 16]); // zero padding (write frontier)

        let out = store
            .process_window(0, 0, 0, window.len() as u64, &window, PIVX_MAGIC, 1000)
            .unwrap();
        assert_eq!(out.blocks_ingested, 2);
        assert_eq!(out.stop, StopReason::Padding);
        // h1 resolves off the canonical parent (10 -> 11); h2 resolves off h1's
        // just-written private record (11 -> 12) — sequential chain resolution.
        assert_eq!(
            store.get_record(&hash1).unwrap().unwrap().claimed_height,
            11
        );
        assert_eq!(
            store.get_record(&hash2).unwrap().unwrap().claimed_height,
            12
        );
        let c = store.load_cursor().unwrap().unwrap();
        assert_eq!(c.cumulative_blocks, 2);
        assert_eq!(c.trailing_hash, hash2);
        assert_eq!(c.offset, (8 + h1.len() + 8 + h2.len()) as u64);
    }

    #[test]
    fn process_window_stops_on_corruption_without_advancing_past_it() {
        let (store, _t) = store_with_chain();
        let parent = [0xAA; 32];
        seed_canonical(&store, 10, parent, true, false);
        let h1 = make_header(1, parent, NBITS);
        let mut window = frame(PIVX_MAGIC, &h1);
        window.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 0]); // non-zero non-magic

        let out = store
            .process_window(0, 0, 0, window.len() as u64, &window, PIVX_MAGIC, 1000)
            .unwrap();
        assert_eq!(out.blocks_ingested, 1);
        assert!(matches!(out.stop, StopReason::Corruption(_)));
        assert_eq!(out.next_offset, (8 + h1.len()) as u64); // cursor stops at the corruption
    }

    #[test]
    fn process_window_stops_torn_mid_record() {
        let (store, _t) = store_with_chain();
        let parent = [0xAA; 32];
        seed_canonical(&store, 10, parent, true, false);
        let h1 = make_header(1, parent, NBITS);
        let h2 = make_header(1, hash_of(&h1), NBITS);
        let mut window = frame(PIVX_MAGIC, &h1);
        let f2 = frame(PIVX_MAGIC, &h2);
        window.extend_from_slice(&f2[..8 + 10]); // h2 header + only 10 body bytes
        let out = store
            .process_window(0, 0, 0, 999, &window, PIVX_MAGIC, 1000)
            .unwrap();
        assert_eq!(out.blocks_ingested, 1);
        assert_eq!(out.stop, StopReason::Torn);
    }

    #[test]
    fn reconcile_demotes_canonical_to_orphan_on_reorg() {
        let (store, _t) = store_with_chain();
        let r = rec(0x77, 0x76, 900, TailState::Canonical);
        commit_block(&store, &r, None);
        set_forward(&store, 900, [0x77; 32]); // canonical
        assert_eq!(store.reconcile_pass(1000, 2000).unwrap().demotions, 0);
        // reorg: a different block now holds height 900
        set_forward(&store, 900, [0x99; 32]);
        let s = store.reconcile_pass(1000, 2000).unwrap();
        assert_eq!(s.demotions, 1);
        assert_eq!(
            store.get_record(&[0x77; 32]).unwrap().unwrap().state,
            TailState::Orphan
        );
    }

    #[test]
    fn reconcile_promotes_orphan_when_its_branch_wins() {
        let (store, _t) = store_with_chain();
        let r = rec(0x88, 0x87, 900, TailState::Orphan);
        commit_block(&store, &r, None);
        set_forward(&store, 900, [0xAA; 32]); // someone else canonical
        assert_eq!(store.reconcile_pass(1000, 2000).unwrap().promotions, 0);
        set_forward(&store, 900, [0x88; 32]); // our branch wins
        let s = store.reconcile_pass(1000, 2000).unwrap();
        assert_eq!(s.promotions, 1);
        assert_eq!(
            store.get_record(&[0x88; 32]).unwrap().unwrap().state,
            TailState::Canonical
        );
    }

    #[test]
    fn reconcile_ages_pending_to_unresolved() {
        let (store, _t) = store_with_chain();
        let mut r = rec(0x66, 0x65, 950, TailState::Pending);
        r.first_seen = 1000;
        commit_block(&store, &r, None); // forward[950] absent
                                        // not yet old enough -> stays pending
        assert_eq!(store.reconcile_pass(1000, 1010).unwrap().unresolved, 0);
        assert_eq!(
            store.get_record(&[0x66; 32]).unwrap().unwrap().state,
            TailState::Pending
        );
        // aged past max_pending_age -> terminal Unresolved
        let s = store
            .reconcile_pass(1000, 1000 + MAX_PENDING_AGE_SECS + 1)
            .unwrap();
        assert_eq!(s.unresolved, 1);
        assert_eq!(
            store.get_record(&[0x66; 32]).unwrap().unwrap().state,
            TailState::Unresolved
        );
    }

    #[test]
    fn snapshot_read_returns_only_orphans() {
        let (store, _t) = store_with_chain();
        commit_block(&store, &rec(0x11, 0x10, 900, TailState::Canonical), None);
        commit_block(&store, &rec(0x22, 0x21, 901, TailState::Orphan), None);
        set_forward(&store, 900, [0x11; 32]); // 0x11 canonical at 900
        set_forward(&store, 901, [0x33; 32]); // a different block canonical at 901 -> 0x22 orphan
        let orphans = store.orphans_under_snapshot().unwrap();
        let hashes: Vec<_> = orphans.iter().map(|r| r.block_hash).collect();
        assert_eq!(hashes, vec![[0x22; 32]]);
    }

    #[test]
    fn prune_evicts_only_settled_records_below_cutoff() {
        let (store, _t) = store_with_chain();
        // tip = 1000, K = 300 -> cutoff height = 700.
        commit_block(&store, &rec(0x01, 0x00, 100, TailState::Canonical), None); // below + settled -> evict
        commit_block(&store, &rec(0x02, 0x00, 200, TailState::Orphan), None); // below + settled -> evict
        commit_block(&store, &rec(0x03, 0x00, 300, TailState::Pending), None); // below but Pending -> keep
        commit_block(&store, &rec(0x04, 0x00, 900, TailState::Canonical), None); // within K -> keep
        let n = store.prune_pass(1000, 10_000).unwrap();
        assert_eq!(n, 2);
        assert!(store.get_record(&[0x01; 32]).unwrap().is_none());
        assert!(store.get_record(&[0x02; 32]).unwrap().is_none());
        assert!(store.get_record(&[0x03; 32]).unwrap().is_some()); // pending never evicted
        assert!(store.get_record(&[0x04; 32]).unwrap().is_some()); // within K of tip
                                                                   // index entries for the evicted records are gone too (scan reflects it).
        let remaining = store.hashes_from_height(i32::MIN).unwrap();
        assert_eq!(remaining.len(), 2);
    }

    #[test]
    fn unknown_bucket_ages_to_unresolved_then_prune_reclaims() {
        let (store, _t) = store_with_chain();
        let mut r = rec(0x5A, 0x59, CLAIMED_UNKNOWN, TailState::Pending);
        r.first_seen = 1000;
        commit_block(&store, &r, None);
        // not yet aged -> stays Pending (and is NOT in the tip-K scan)
        store.reconcile_pass(2000, 1010).unwrap();
        assert_eq!(
            store.get_record(&[0x5A; 32]).unwrap().unwrap().state,
            TailState::Pending
        );
        // aged past max_pending_age -> terminal Unresolved (the leak-closing transition)
        let s = store
            .reconcile_pass(2000, 1000 + MAX_PENDING_AGE_SECS + 1)
            .unwrap();
        assert_eq!(s.unresolved, 1);
        assert_eq!(
            store.get_record(&[0x5A; 32]).unwrap().unwrap().state,
            TailState::Unresolved
        );
        // prune reclaims the terminal Unresolved row -> growth is bounded
        assert_eq!(store.prune_pass(2000, 100).unwrap(), 1);
        assert!(store.get_record(&[0x5A; 32]).unwrap().is_none());
    }

    #[test]
    fn unknown_bucket_resolves_when_parent_appears() {
        let (store, _t) = store_with_chain();
        let r = rec(0x88, 0x77, CLAIMED_UNKNOWN, TailState::Pending); // prev = [0x77;32]
        commit_block(&store, &r, None);
        // The parent later becomes canonical; the child should resolve + classify.
        seed_canonical(&store, 500, [0x77; 32], true, false);
        seed_canonical(&store, 501, [0x88; 32], false, false); // child wins height 501
        store.reconcile_pass(600, 2000).unwrap();
        let got = store.get_record(&[0x88; 32]).unwrap().unwrap();
        assert_eq!(got.claimed_height, 501);
        assert_eq!(got.state, TailState::Canonical);
    }

    // ---- 3d async tick layer (tokio) ----

    fn write_blk(dir: &Path, file_no: u32, bytes: &[u8]) {
        std::fs::write(dir.join(format!("blk{file_no:05}.dat")), bytes).unwrap();
    }

    fn seed_cursor_file0(store: &TailStore) {
        let mut batch = rocksdb::WriteBatch::default();
        store
            .stage_cursor(
                &mut batch,
                &TailCursor {
                    file_no: 0,
                    offset: 0,
                    trailing_hash: [0u8; 32],
                    cumulative_blocks: 0,
                    file_len_watermark: 0,
                },
            )
            .unwrap();
        store.commit(batch).unwrap();
    }

    #[tokio::test]
    async fn tick_drains_file_and_is_idempotent() {
        let (store, _dbt) = store_with_chain();
        let bdir = tempfile::TempDir::new().unwrap();
        let parent = [0xAA; 32];
        seed_canonical(&store, 10, parent, true, false);
        let h1 = make_header(1, parent, NBITS);
        let h2 = make_header(1, hash_of(&h1), NBITS);
        let mut content = frame(PIVX_MAGIC, &h1);
        content.extend_from_slice(&frame(PIVX_MAGIC, &h2));
        content.extend_from_slice(&[0u8; 32]); // trailing padding
        write_blk(bdir.path(), 0, &content);

        let n = tail_read_tick(&store, bdir.path(), PIVX_MAGIC, 1 << 20)
            .await
            .unwrap();
        assert_eq!(n, 2);
        let c = store.load_cursor().unwrap().unwrap();
        assert_eq!((c.file_no, c.cumulative_blocks), (0, 2));
        // re-read with no new data -> no-op
        assert_eq!(
            tail_read_tick(&store, bdir.path(), PIVX_MAGIC, 1 << 20)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn tick_halts_on_file_shrink() {
        let (store, _dbt) = store_with_chain();
        let bdir = tempfile::TempDir::new().unwrap();
        let parent = [0xAA; 32];
        seed_canonical(&store, 10, parent, true, false);
        let mut content = frame(PIVX_MAGIC, &make_header(1, parent, NBITS));
        content.extend_from_slice(&[0u8; 32]);
        write_blk(bdir.path(), 0, &content);
        tail_read_tick(&store, bdir.path(), PIVX_MAGIC, 1 << 20)
            .await
            .unwrap();
        // file shrinks below the watermark (reindex/truncate) -> halt, no advance
        write_blk(bdir.path(), 0, &content[..content.len() / 2]);
        assert_eq!(
            tail_read_tick(&store, bdir.path(), PIVX_MAGIC, 1 << 20)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn tick_rolls_over_when_full_file_and_next_exists() {
        let (store, _dbt) = store_with_chain();
        let bdir = tempfile::TempDir::new().unwrap();
        let parent = [0xAA; 32];
        seed_canonical(&store, 10, parent, true, false);
        let mut content = frame(PIVX_MAGIC, &make_header(1, parent, NBITS));
        content.extend_from_slice(&[0u8; 32]); // trailing padding = "full" file
        write_blk(bdir.path(), 0, &content);
        write_blk(bdir.path(), 1, &[]); // next file exists
                                        // start the tail on file 0 (a no-cursor start would pick the highest = blk1).
        seed_cursor_file0(&store);
        tail_read_tick(&store, bdir.path(), PIVX_MAGIC, 1 << 20)
            .await
            .unwrap();
        let c = store.load_cursor().unwrap().unwrap();
        assert_eq!((c.file_no, c.offset), (1, 0));
    }

    #[tokio::test]
    async fn tick_does_not_roll_over_on_torn_trailing_record() {
        // Regression for the rollover-skip bug: a Torn (still-flushing) trailing
        // record must NOT advance to the next file even if blk{n+1} exists.
        let (store, _dbt) = store_with_chain();
        let bdir = tempfile::TempDir::new().unwrap();
        let parent = [0xAA; 32];
        seed_canonical(&store, 10, parent, true, false);
        let h1 = make_header(1, parent, NBITS);
        let h2 = make_header(1, hash_of(&h1), NBITS);
        let mut content = frame(PIVX_MAGIC, &h1);
        let f2 = frame(PIVX_MAGIC, &h2);
        content.extend_from_slice(&f2[..8 + 10]); // h2 torn
        write_blk(bdir.path(), 0, &content);
        write_blk(bdir.path(), 1, &[]); // next exists
        seed_cursor_file0(&store); // start on file 0 (not the highest, empty blk1)
        let n = tail_read_tick(&store, bdir.path(), PIVX_MAGIC, 1 << 20)
            .await
            .unwrap();
        assert_eq!(n, 1); // only h1 ingested
        assert_eq!(store.load_cursor().unwrap().unwrap().file_no, 0); // stayed — waits for h2
    }
}
