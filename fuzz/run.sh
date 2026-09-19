#!/usr/bin/env bash
# Runs one fuzz target for a bounded time, in a single process, on the committed seeds plus
# whatever corpus earlier runs left in fuzz/corpus/<target>.
#
#   fuzz/run.sh <target> [seconds]      # default 60; `cargo fuzz list` names the targets
#
# Needs a nightly toolchain and cargo-fuzz (see fuzz/README.md). A crash stops the run, exits
# non-zero and leaves the input in fuzz/artifacts/<target>/.
set -euo pipefail

target=${1:?usage: fuzz/run.sh <target> [seconds]}
seconds=${2:-60}
fuzz=$(cd "$(dirname "$0")" && pwd)

mkdir -p "$fuzz/corpus/$target"
cd "$fuzz/.."
exec cargo +"${FUZZ_TOOLCHAIN:-nightly}" fuzz run "$target" \
    "$fuzz/corpus/$target" "$fuzz/seeds/$target" -- \
    -max_total_time="$seconds" \
    -dict="$fuzz/dict/jev.dict" \
    -timeout=10 \
    -rss_limit_mb=2048 \
    -print_final_stats=1
