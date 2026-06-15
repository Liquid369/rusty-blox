#!/usr/bin/env bash
#
# Full-sync RAM + time benchmark for rusty-blox.
#
# Runs a FRESH full sync (leveldb import -> .blk parse -> height resolution ->
# address enrichment) into an isolated temp DB and reports per-phase wall-time
# and peak RSS, with an optional duddino balance reconcile.
#
# Production-safe: uses its own db_path, port, and /tmp copy dirs, tracks the
# exact child PID it spawned, and never `pkill`s rustyblox by name (that would
# kill a live explorer). It stops only the process / container it started.
#
# Two modes:
#   native (default) - runs target/release/rustyblox locally. Representative
#       TIMING (leveldb is read from local disk). RSS is sampled from `ps`; on
#       macOS jemalloc RSS swings ~+-5g run-to-run, so read the trend.
#   docker           - builds the image and runs it with the PIVX blocks bind-
#       mounted read-only; reads cgroup v2 memory.peak (authoritative full-sync
#       peak) + memory.current (phase split). Representative RAM on real Linux.
#       NOTE: in a small Docker VM the sync may OOM before finishing -- give the
#       engine >= 10 GB. Docker file-sharing inflates the mounted-leveldb read,
#       so docker-mode TIMING is NOT representative; use native mode for timing.
#
# Usage:   scripts/bench-sync.sh [native|docker]
# Env:
#   BENCH_PORT=3099   BENCH_SAMPLE=2(s)   BENCH_RECONCILE=1   BENCH_KEEP_DB=0
#   BENCH_DIR=/tmp/rbx_bench   BENCH_MEM=10g(docker)   BENCH_IMAGE=rustyblox:bench
#   BENCH_BLOCKS="$HOME/Library/Application Support/PIVX/blocks"
set -uo pipefail

MODE="${1:-native}"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$REPO/target/release/rustyblox"
PORT="${BENCH_PORT:-3099}"
SAMPLE="${BENCH_SAMPLE:-2}"
RECONCILE="${BENCH_RECONCILE:-1}"
KEEP_DB="${BENCH_KEEP_DB:-0}"
SCRATCH="${BENCH_DIR:-/tmp/rbx_bench}"
BLOCKS="${BENCH_BLOCKS:-$HOME/Library/Application Support/PIVX/blocks}"
RANK1="DU8gPC5mh4KxWJARQRxoESFark2jAguBr5"   # duddino reconcile anchor
API="http://127.0.0.1:$PORT/api/v2"
GB() { awk "BEGIN{printf \"%.2f\", ${1:-0}/1073741824}"; }   # bytes -> GiB

# ---- phase timing + RSS + reconcile report (shared by both modes) -----------
# args: <run.log> <pre_enrich_peak_bytes> <enrich_peak_bytes> <full_peak_bytes|"">
report() {
  local logf="$1" pre="$2" enr="$3" full="${4:-}"
  [ -z "$full" ] && full=$(( pre > enr ? pre : enr ))
  echo ""
  echo "================ rusty-blox full-sync benchmark ($MODE) ================"
  python3 - "$logf" <<'PY'
import re,sys
from datetime import datetime
def ts(s):
    m=re.search(r'\d{4}-\d\d-\d\dT[\d:.]+', re.sub(r'\x1b\[[0-9;]*m','',s))
    return m.group(0) if m else None
M=['START','Starting parallel file processing','Leveldb import complete - switching',
   'Building address index from transactions','Pass 1 complete','Pass 2 complete',
   'Pass 2b complete','Address index building complete']
marks={};first=None
for l in open(sys.argv[1],encoding='utf-8',errors='ignore'):
    t=ts(l)
    if t and not first: first=t; marks['START']=t
    for m in M[1:]:
        if m in l and m not in marks: marks[m]=t
def d(a,b):
    try: return (datetime.fromisoformat(marks[b])-datetime.fromisoformat(marks[a])).total_seconds()
    except: return None
def row(lbl,a,b):
    v=d(a,b)
    if v is not None: print(f"  {lbl:<34} {v:7.0f}s  ({v/60:5.1f}m)")
print("  PHASE                               WALL-TIME")
print("  " + "-"*46)
row("leveldb import (+ chainwork)","START","Starting parallel file processing")
row(".blk parse (+ resolve heights)","Starting parallel file processing","Leveldb import complete - switching")
row("height resolution","Leveldb import complete - switching","Building address index from transactions")
row("enrichment pass 1","Building address index from transactions","Pass 1 complete")
row("enrichment pass 2","Pass 1 complete","Pass 2 complete")
row("enrichment pass 2b","Pass 2 complete","Pass 2b complete")
row("enrichment write + hodl","Pass 2b complete","Address index building complete")
print("  " + "-"*46)
tot=d("START","Address index building complete")
if tot is not None: print(f"  {'TOTAL (-> addr-index built)':<34} {tot:7.0f}s  ({tot/60:5.1f}m)  + persist")
PY
  echo "  ----------------------------------------------"
  echo "  PEAK RSS   pre-enrichment : $(GB "$pre") GiB   (leveldb / parse / resolve)"
  echo "             enrichment     : $(GB "$enr") GiB"
  echo "             FULL SYNC      : $(GB "$full") GiB"
  if [ "$RECONCILE" = 1 ] && [ -s "$SCRATCH/rank1.json" ]; then
    local b dud hodl
    b=$(python3 -c "import json;d=json.load(open('$SCRATCH/rank1.json'));print(d.get('balance'),d.get('totalReceived'),d.get('totalSent'))" 2>/dev/null)
    dud=$(curl -s -m12 "https://explorer.duddino.com/api/v2/address/$RANK1" | python3 -c "import sys,json;d=json.load(sys.stdin);print(d.get('balance'),d.get('totalReceived'),d.get('totalSent'))" 2>/dev/null)
    hodl=$(python3 -c "import json;d=json.load(open('$SCRATCH/hodl.json'));print('%.2f'%sum(float(x['value']) for x in d['bands']))" 2>/dev/null)
    echo "  ----------------------------------------------"
    echo "  RECONCILE  rank-1 local  : $b"
    echo "             rank-1 duddino: $dud"
    echo "             HODL total    : $hodl  (correct ~103M)"
    [ -n "$b" ] && [ "$b" = "$dud" ] && echo "             => BYTE-EXACT MATCH" || echo "             => MISMATCH / no data (investigate)"
  fi
  echo "========================================================================"
}

in_enrich() { grep -q "Building address index from transactions" "$SCRATCH/run.log" 2>/dev/null; }
done_sync()  { grep -q "Address index building complete" "$SCRATCH/run.log" 2>/dev/null; }

# ============================ NATIVE MODE ===================================
run_native() {
  [ -x "$BIN" ] || { echo "build first:  cargo +1.88.0 build --release --offline --locked --bin rustyblox"; exit 1; }
  command -v lsof >/dev/null && lsof -ti:"$PORT" 2>/dev/null | xargs kill -9 2>/dev/null
  rm -rf "$SCRATCH"; mkdir -p "$SCRATCH/data"
  sed -e 's#^db_path = .*#db_path = "'"$SCRATCH"'/data/blocks.db"#' \
      -e 's#^port = .*#port = '"$PORT"'#' \
      -e 's#^block_index_copy_dir = .*#block_index_copy_dir = "'"$SCRATCH"'/bi_copy"#' \
      -e 's#^chainstate_copy_dir = .*#chainstate_copy_dir = "'"$SCRATCH"'/cs_copy"#' \
      "$REPO/config.toml" > "$SCRATCH/config.toml"
  # subshell + exec => $! is the exact rustyblox PID (no `time` wrapper, no
  # confusion with a production rustyblox running elsewhere).
  ( cd "$SCRATCH" && exec env RUST_LOG=info "$BIN" ) > "$SCRATCH/run.log" 2>&1 &
  local pid=$! pre=0 enr=0 rss
  echo "[bench] native sync started (pid $pid) -> $SCRATCH (port $PORT). sampling RSS every ${SAMPLE}s ..."
  while kill -0 "$pid" 2>/dev/null; do
    rss=$(( $(ps -o rss= -p "$pid" 2>/dev/null | tr -d ' ' || echo 0) * 1024 ))   # KiB -> bytes
    if in_enrich; then [ "$rss" -gt "$enr" ] && enr=$rss; else [ "$rss" -gt "$pre" ] && pre=$rss; fi
    if done_sync; then
      if [ "$RECONCILE" = 1 ]; then
        # The log says "complete" a moment before the API can serve the address
        # index, so a single poll usually misses. Retry until the address
        # endpoint returns a balance (or give up after ~60s).
        for _ in $(seq 1 20); do
          if curl -s -m5 "$API/address/$RANK1" 2>/dev/null | grep -q '"balance"'; then
            curl -s -m8 "$API/address/$RANK1" > "$SCRATCH/rank1.json"
            curl -s -m8 "$API/analytics/hodl"  > "$SCRATCH/hodl.json"
            break
          fi
          sleep 3
        done
      fi
      sleep "$SAMPLE"
      rss=$(( $(ps -o rss= -p "$pid" 2>/dev/null | tr -d ' ' || echo 0) * 1024 )); [ "$rss" -gt "$enr" ] && enr=$rss
      kill -TERM "$pid" 2>/dev/null; sleep 3; break
    fi
    sleep "$SAMPLE"
  done
  report "$SCRATCH/run.log" "$pre" "$enr" ""
  [ "$KEEP_DB" = 1 ] || rm -rf "$SCRATCH/data"
}

# ============================ DOCKER MODE ===================================
run_docker() {
  command -v docker >/dev/null || { echo "docker not found"; exit 1; }
  [ -d "$BLOCKS/index" ] || { echo "[bench] PIVX block index not found at $BLOCKS/index"; exit 1; }
  local img="${BENCH_IMAGE:-rustyblox:bench}" mem="${BENCH_MEM:-10g}" cname="rbx-bench"
  echo "[bench] building $img (this can take a while) ..."
  docker build -t "$img" "$REPO" >/dev/null || { echo "docker build failed"; exit 1; }
  docker rm -f "$cname" >/dev/null 2>&1
  rm -rf "$SCRATCH"; mkdir -p "$SCRATCH"
  # /data lives on a VM-internal Docker volume, NOT a host bind mount: RocksDB
  # writes to fast ext4 inside the VM so writeback is prompt and dirty pages
  # reclaim normally. A virtiofs bind mount for /data accumulates unreclaimable
  # dirty pages under the cgroup and OOMs the sync at the parse->resolve boundary
  # -- a measurement artifact, not real workload memory.
  docker volume rm rbx-bench-data >/dev/null 2>&1; docker volume create rbx-bench-data >/dev/null
  sed -e 's#^db_path = .*#db_path = "/data/blocks.db"#' \
      -e 's#^blk_dir = .*#blk_dir = "/pivx/blocks"#' \
      -e 's#^pivx_data_dir = .*#pivx_data_dir = "/pivx"#' \
      -e 's#^port = .*#port = 3005#' \
      "$REPO/config.toml" > "$SCRATCH/config.toml"
  docker run -d --name "$cname" --memory="$mem" --memory-swap="$mem" \
    -v "$BLOCKS:/pivx/blocks:ro" -v rbx-bench-data:/data -v "$SCRATCH/config.toml:/app/config.toml:ro" \
    -e RUST_LOG=info "$img" >/dev/null
  echo "[bench] docker sync started ($cname, mem=$mem). tracking ANON (hard req) + cgroup total ..."
  local pre=0 enr=0 apre=0 aenr=0 cur an full running
  while true; do
    running=$(docker inspect -f '{{.State.Running}}' "$cname" 2>/dev/null)
    docker logs "$cname" > "$SCRATCH/run.log" 2>&1
    cur=$(docker exec "$cname" cat /sys/fs/cgroup/memory.current 2>/dev/null || echo 0)
    an=$(docker exec "$cname" sh -c "grep '^anon ' /sys/fs/cgroup/memory.stat | cut -d' ' -f2" 2>/dev/null || echo 0)
    if in_enrich; then
      [ "$cur" -gt "$enr" ] 2>/dev/null && enr=$cur; [ "$an" -gt "$aenr" ] 2>/dev/null && aenr=$an
    else
      [ "$cur" -gt "$pre" ] 2>/dev/null && pre=$cur; [ "$an" -gt "$apre" ] 2>/dev/null && apre=$an
    fi
    if [ "$running" != "true" ]; then
      echo "[bench] container exited: $(docker inspect -f 'exit={{.State.ExitCode}} oom={{.State.OOMKilled}}' "$cname")"
      full=""   # container stopped; memory.peak no longer readable -> report uses max(pre,enr)
      break
    fi
    if done_sync; then
      sleep "$SAMPLE"; cur=$(docker exec "$cname" cat /sys/fs/cgroup/memory.current 2>/dev/null||echo 0); [ "$cur" -gt "$enr" ] 2>/dev/null && enr=$cur
      an=$(docker exec "$cname" sh -c "grep '^anon ' /sys/fs/cgroup/memory.stat|cut -d' ' -f2" 2>/dev/null||echo 0); [ "$an" -gt "$aenr" ] 2>/dev/null && aenr=$an
      full=$(docker exec "$cname" cat /sys/fs/cgroup/memory.peak 2>/dev/null || echo 0)
      if [ "$RECONCILE" = 1 ]; then
        docker exec "$cname" curl -s -m8 "localhost:3005/api/v2/address/$RANK1" > "$SCRATCH/rank1.json" 2>/dev/null
        docker exec "$cname" curl -s -m8 "localhost:3005/api/v2/analytics/hodl" > "$SCRATCH/hodl.json" 2>/dev/null
      fi
      docker logs "$cname" > "$SCRATCH/run.log" 2>&1; docker stop -t 5 "$cname" >/dev/null
      break
    fi
    sleep "$SAMPLE"
  done
  report "$SCRATCH/run.log" "$pre" "$enr" "${full:-}"
  local ahard=$(( apre > aenr ? apre : aenr ))
  echo "  ANON (hard req)  pre-enrichment : $(GB "$apre") GiB   <- the number that matters"
  echo "                   enrichment     : $(GB "$aenr") GiB"
  echo "                   FULL SYNC      : $(GB "$ahard") GiB   (anonymous working set; drives OOM)"
  echo "  NOTE: the 'PEAK RSS' above includes page cache that macOS Docker virtiofs"
  echo "        bind-mounts (/pivx, and /data unless a volume) do NOT reclaim, so it"
  echo "        over-reports here. ANON is the real RAM the sync needs; on a real"
  echo "        Linux host (local disk) the cgroup total tracks anon + reclaimable cache."
  echo "  ========================================================================"
  docker rm -f "$cname" >/dev/null 2>&1
  [ "$KEEP_DB" = 1 ] || docker volume rm rbx-bench-data >/dev/null 2>&1
  echo "[bench] (docker-mode timing is leveldb-file-sharing-inflated; use native mode for timing)"
}

case "$MODE" in
  native) run_native ;;
  docker) run_docker ;;
  *) echo "usage: $0 [native|docker]"; exit 1 ;;
esac
