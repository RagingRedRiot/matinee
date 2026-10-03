#!/bin/bash
# Fails until its PASS_ON-th attempt, the way a test that trips over a
# stale cache does.
state="$HOME/.local/state/pipeline"
mkdir -p "$state"
attempt=$(( $(cat "$state/$JOB-test" 2>/dev/null || echo 0) + 1 ))
echo "$attempt" > "$state/$JOB-test"
echo "running 214 tests (attempt $attempt)"
sleep 1.5
if [ "$attempt" -lt "${PASS_ON:-1}" ]; then
  echo "3 failed: test_cache_eviction, test_cache_ttl, test_cache_warmup" >&2
  exit 1
fi
echo "214 passed"
