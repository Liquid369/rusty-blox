// Reads rejected budget votes back from the node over P2P. Core inserts each
// incoming vote into mapSeenProposalVotes before the signature check and
// UpdateProposal (budgetmanager.cpp:1152), and answers getdata for it
// (net_processing.cpp:998) until ReloadMapSeen drops it (budgetmanager.cpp:31).
// So votes rejected as older or too soon, whose log line carries only the
// hash, can be recovered whole. Read-only: handshake, getdata, read, close.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::gov::store::{Hash, Outpoint};

const MAGIC: [u8; 4] = [0x90, 0xc4, 0xfd, 0xe9];
const PROTOCOL_VERSION: i32 = 70928;
/// GetDataMsg::MSG_BUDGET_VOTE (protocol.h:446).
const MSG_BUDGET_VOTE: u32 = 9;
const BATCH: usize = 500;
/// Core's own inbound cap (net.cpp MAX_PROTOCOL_MESSAGE_LENGTH, 2 MiB).
const MAX_MESSAGE: usize = 2 * 1024 * 1024;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RecoveredVote {
    /// Collateral txid (display order) and output index.
    pub mn_txid: Hash,
    pub mn_n: u32,
    pub proposal: Hash,
    /// Core nVote, 4 bytes on the wire; not range-checked by Core.
    pub vote: u32,
    pub n_time: i64,
    /// Empty scriptSig and final nSequence, as Core builds a vote's vin. A
    /// legacy string-signed vote can differ without breaking its signature.
    pub canonical: bool,
}

impl RecoveredVote {
    /// As gov.db keys a collateral: txid || vout u32 BE.
    pub fn outpoint(&self) -> Outpoint {
        let mut o = [0u8; 36];
        o[..32].copy_from_slice(&self.mn_txid);
        o[32..].copy_from_slice(&self.mn_n.to_be_bytes());
        o
    }
}

fn dsha(b: &[u8]) -> [u8; 32] {
    let once: [u8; 32] = Sha256::digest(b).into();
    Sha256::digest(&once).into()
}

fn rev(mut h: [u8; 32]) -> [u8; 32] {
    h.reverse();
    h
}

fn varint(n: usize, out: &mut Vec<u8>) {
    match n {
        0..=0xfc => out.push(n as u8),
        0xfd..=0xffff => {
            out.push(0xfd);
            out.extend_from_slice(&(n as u16).to_le_bytes());
        }
        _ => {
            out.push(0xfe);
            out.extend_from_slice(&(n as u32).to_le_bytes());
        }
    }
}

fn read_varint(b: &[u8], i: &mut usize) -> Option<usize> {
    let first = *b.get(*i)?;
    *i += 1;
    let (len, v) = match first {
        0xfd => (
            2,
            u16::from_le_bytes(b.get(*i..*i + 2)?.try_into().ok()?) as usize,
        ),
        0xfe => (
            4,
            u32::from_le_bytes(b.get(*i..*i + 4)?.try_into().ok()?) as usize,
        ),
        0xff => return None,
        n => (0, n as usize),
    };
    *i += len;
    Some(v)
}

fn frame(cmd: &str, payload: &[u8]) -> Vec<u8> {
    let mut m = MAGIC.to_vec();
    let mut c = [0u8; 12];
    c[..cmd.len()].copy_from_slice(cmd.as_bytes());
    m.extend_from_slice(&c);
    m.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    m.extend_from_slice(&dsha(payload)[..4]);
    m.extend_from_slice(payload);
    m
}

fn version_msg() -> Vec<u8> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let addr = |p: &mut Vec<u8>| {
        p.extend_from_slice(&0u64.to_le_bytes());
        p.extend_from_slice(&[0u8; 10]);
        p.extend_from_slice(&[0xff, 0xff, 127, 0, 0, 1, 0, 0]);
    };
    let mut p = Vec::new();
    p.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    p.extend_from_slice(&0u64.to_le_bytes());
    p.extend_from_slice(&now.to_le_bytes());
    addr(&mut p);
    addr(&mut p);
    p.extend_from_slice(&rand_nonce().to_le_bytes());
    let ua = b"/rustyblox-govlog/";
    varint(ua.len(), &mut p);
    p.extend_from_slice(ua);
    p.extend_from_slice(&0i32.to_le_bytes());
    // fRelay = false: no tx inv traffic on this short-lived connection.
    p.push(0);
    p
}

fn rand_nonce() -> u64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    t ^ u64::from(std::process::id()).rotate_left(32)
}

/// Decodes an `mvote` payload (CBudgetVote: vin, proposal, nVote u32, nTime,
/// vchSig, nMessVersion). The hash is SHA256d over the received vin..nTime
/// bytes, i.e. Core's GetHash whatever the vin looks like.
pub fn decode_vote(p: &[u8]) -> Option<(Hash, RecoveredVote)> {
    let txid: [u8; 32] = p.get(..32)?.try_into().ok()?;
    let n = u32::from_le_bytes(p.get(32..36)?.try_into().ok()?);
    let mut i = 36;
    let script = read_varint(p, &mut i)?;
    i += script;
    let seq = u32::from_le_bytes(p.get(i..i + 4)?.try_into().ok()?);
    i += 4;
    let proposal = rev(p.get(i..i + 32)?.try_into().ok()?);
    i += 32;
    let vote = u32::from_le_bytes(p.get(i..i + 4)?.try_into().ok()?);
    i += 4;
    let n_time = i64::from_le_bytes(p.get(i..i + 8)?.try_into().ok()?);
    i += 8;
    Some((
        rev(dsha(&p[..i])),
        RecoveredVote {
            mn_txid: rev(txid),
            mn_n: n,
            proposal,
            vote,
            n_time,
            canonical: script == 0 && seq == u32::MAX,
        },
    ))
}

/// Outcome per requested hash: Some(vote) if the node returned it, None if it
/// answered notfound. Hashes it never answered before `timeout` are absent.
pub fn fetch(
    addr: &str,
    hashes: &[Hash],
    timeout: Duration,
) -> Result<HashMap<Hash, Option<RecoveredVote>>, String> {
    let deadline = Instant::now() + timeout;
    // ponytail: name resolution has no timeout; pass an IP literal, as the
    // unit file does.
    let sock = addr
        .to_socket_addrs()
        .map_err(|e| format!("{addr}: {e}"))?
        .next()
        .ok_or(format!("{addr}: no address"))?;
    let mut s = TcpStream::connect_timeout(&sock, timeout).map_err(|e| format!("{addr}: {e}"))?;
    s.set_read_timeout(Some(Duration::from_millis(500)))
        .map_err(|e| e.to_string())?;
    // A peer that stops reading cannot hold the sidecar loop past `timeout`.
    s.set_write_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    s.write_all(&frame("version", &version_msg()))
        .map_err(|e| e.to_string())?;
    let wanted: HashSet<Hash> = hashes.iter().copied().collect();
    let mut out: HashMap<Hash, Option<RecoveredVote>> = HashMap::new();
    let mut buf: Vec<u8> = Vec::new();
    let mut sent = false;
    // Commands the peer sent, so a handshake that never completes says why.
    let mut seen: Vec<String> = Vec::new();
    let mut chunk = [0u8; 65536];
    while Instant::now() < deadline && out.len() < wanted.len() {
        match s.read(&mut chunk) {
            Ok(0) => return Err(format!("{addr}: closed by peer")),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(e.to_string()),
        }
        while buf.len() >= 24 {
            if buf[..4] != MAGIC {
                return Err(format!("{addr}: bad magic"));
            }
            let len = u32::from_le_bytes(buf[16..20].try_into().unwrap_or_default()) as usize;
            if len > MAX_MESSAGE {
                return Err(format!("{addr}: {len}-byte message over the 2 MiB cap"));
            }
            if buf.len() < 24 + len {
                break;
            }
            if dsha(&buf[24..24 + len])[..4] != buf[20..24] {
                return Err(format!("{addr}: bad checksum"));
            }
            let cmd: String = buf[4..16]
                .iter()
                .take_while(|b| **b != 0)
                .map(|b| *b as char)
                .collect();
            let payload: Vec<u8> = buf[24..24 + len].to_vec();
            buf.drain(..24 + len);
            if !seen.contains(&cmd) {
                seen.push(cmd.clone());
            }
            match cmd.as_str() {
                "version" => s
                    .write_all(&frame("verack", &[]))
                    .map_err(|e| e.to_string())?,
                "ping" => s
                    .write_all(&frame("pong", &payload))
                    .map_err(|e| e.to_string())?,
                "verack" if !sent => {
                    for c in hashes.chunks(BATCH) {
                        let mut p = Vec::new();
                        varint(c.len(), &mut p);
                        for h in c {
                            p.extend_from_slice(&MSG_BUDGET_VOTE.to_le_bytes());
                            p.extend_from_slice(&rev(*h));
                        }
                        s.write_all(&frame("getdata", &p))
                            .map_err(|e| e.to_string())?;
                    }
                    sent = true;
                }
                "mvote" => {
                    // A payload whose hash was not requested is ignored.
                    if let Some((h, v)) = decode_vote(&payload).filter(|(h, _)| wanted.contains(h))
                    {
                        out.insert(h, Some(v));
                    }
                }
                "notfound" => {
                    let mut i = 0;
                    let n = read_varint(&payload, &mut i).unwrap_or(0);
                    for _ in 0..n {
                        let (Some(t), Some(h)) =
                            (payload.get(i..i + 4), payload.get(i + 4..i + 36))
                        else {
                            break;
                        };
                        if t == MSG_BUDGET_VOTE.to_le_bytes() {
                            if let Ok(h) = <[u8; 32]>::try_from(h).map(rev) {
                                if wanted.contains(&h) {
                                    out.entry(h).or_insert(None);
                                }
                            }
                        }
                        i += 36;
                    }
                }
                _ => {}
            }
        }
    }
    // Core answers every getdata item with the vote or notfound, so an
    // empty result here is not "none found": the request never went out.
    if !sent {
        return Err(format!(
            "{addr}: handshake not completed in {timeout:?}; peer sent [{}]",
            seen.join(", ")
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gov::store::parse_hash;

    /// A vote the wallet node rejected on 2026-10-21 as older than stored,
    /// read back by getdata on 2026-10-22.
    #[test]
    fn decodes_a_real_rejected_vote() {
        let p = format!(
            "{}/tests/fixtures/gov/mvote_older_2026-10-21.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let fx: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        let payload = hex::decode(fx["payload_hex"].as_str().unwrap()).unwrap();
        let (h, v) = decode_vote(&payload).unwrap();
        assert_eq!(h, parse_hash(fx["hash"].as_str().unwrap()).unwrap());
        assert_eq!(
            crate::gov::votematch::outpoint_str(&v.outpoint()),
            "cf2f710568dc298c9fee9391be96731fea12db9b58ad632c7b39d6b299363847-47"
        );
        assert_eq!(
            hex::encode(v.proposal),
            "71f8dc4e21a11740bc7d9478b3c28b73d622a4262882d7854fd358595ac75aef"
        );
        assert_eq!((v.vote, v.n_time, v.canonical), (1, 1_789_731_992, true));
        // The canonical model in votematch agrees for a canonical vin.
        assert_eq!(
            crate::gov::votematch::vote_hash(&v.outpoint(), &v.proposal, v.vote as u8, v.n_time),
            h
        );
        assert!(decode_vote(&payload[..60]).is_none());
    }

    /// A peer that accepts and never completes the handshake is an error,
    /// not an empty "nothing found" (the sidecar would spend retries on it).
    #[test]
    fn silent_peer_is_an_error() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let hold = std::thread::spawn(move || l.accept().map(|(s, _)| s));
        let err = fetch(&addr, &[[1u8; 32]], Duration::from_millis(800)).unwrap_err();
        assert!(err.contains("handshake not completed"), "{err}");
        drop(hold.join());
    }
}
