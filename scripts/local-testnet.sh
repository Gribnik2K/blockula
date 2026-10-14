#!/usr/bin/env bash
# Local multi-node smoke test: 4 validators (simplex BFT over authenticated p2p) + 1 full node
# that joins over HTTP. Runs the SDK e2e against a validator and against the full node, restarts
# a validator mid-run, then checks every node agrees on block hashes and state roots.
#
#   scripts/local-testnet.sh            # build release, run, tear down
#   KEEP=1 scripts/local-testnet.sh     # leave the testnet running afterwards
#   BIN=node/target/debug/blockula scripts/local-testnet.sh
set -euo pipefail
cd "$(dirname "$0")/.."

N=${N:-4}
DIR=${DIR:-$(mktemp -d "${TMPDIR:-/tmp}/blockula-testnet.XXXXXX")}
BIN=${BIN:-node/target/release/blockula}
FULL_PORT=${FULL_PORT:-7171}
PIDS=()

cleanup() {
  if [[ -z "${KEEP:-}" ]]; then
    for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null || true; done
    wait 2>/dev/null || true
  else
    echo "🧛 still running (KEEP=1). Data + logs: $DIR"
  fi
}
trap cleanup EXIT

[[ -n "${SKIP_BUILD:-}" ]] || cargo build --release --manifest-path node/Cargo.toml
echo "📁 $DIR"

"$BIN" summon testnet --home "$DIR" --validators "$N"
addr() { node -e "console.log(require('$DIR/genesis.json').validators[$1].http_addr)"; }
status() { curl -sf -m 3 "http://$1/api/status"; }
field() { node -e "let s='';process.stdin.on('data',d=>s+=d).on('end',()=>console.log(JSON.parse(s)$1))"; }

start_validator() {
  "$BIN" run --home "$DIR/v$1" >>"$DIR/v$1.log" 2>&1 &
  PIDS[$1]=$!
}
for i in $(seq 0 $((N - 1))); do start_validator "$i"; done

wait_height() { # url height
  for _ in $(seq 1 120); do
    h=$(status "$1" 2>/dev/null | field .height 2>/dev/null || true); h=${h:-0}
    [[ "$h" -ge "$2" ]] && return 0
    sleep 1
  done
  echo "❌ $1 never reached height $2" >&2; exit 1
}
V0=$(addr 0); V2=$(addr 2)
wait_height "$V0" 3
echo "✓ validators are finalizing blocks"

"$BIN" join --genesis "http://$V0" --home "$DIR/full" --port "$FULL_PORT" >"$DIR/full.log" 2>&1 &
PIDS[$N]=$!
FULL="127.0.0.1:$FULL_PORT"

echo "— SDK e2e against validator 2 (faucet + txs gossiped to whoever leads)"
BLOCKULA_RPC="http://$V2" npm test -w @blockula/sdk

echo "— restarting validator 1"
kill "${PIDS[1]}"; wait "${PIDS[1]}" 2>/dev/null || true
sleep 5
start_validator 1

wait_height "$FULL" "$(status "$V0" | field .height)"
echo "— SDK e2e against the full node (txs + faucet forwarded to validators)"
BLOCKULA_RPC="http://$FULL" npm test -w @blockula/sdk

# Agreement: same hash and state root at a common height on every node.
NODES=()
for i in $(seq 0 $((N - 1))); do NODES+=("$(addr "$i")"); done
NODES+=("$FULL")
target=$(status "$V0" | field .height)
for n in "${NODES[@]}"; do wait_height "$n" "$target"; done
ref=""
for n in "${NODES[@]}"; do
  line=$(curl -sf "http://$n/api/block/$target" | field '.hash+" "+JSON.parse(s).state_root')
  echo "  $n  #$target  $line"
  [[ -z "$ref" ]] && ref=$line
  [[ "$line" == "$ref" ]] || { echo "❌ nodes disagree at height $target" >&2; exit 1; }
done
echo "✓ all $((N + 1)) nodes agree on block #$target (hash + state root)"
