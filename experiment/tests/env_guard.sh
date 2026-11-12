#!/usr/bin/env bash
# Environment guard test. Run from the repo root: bash experiment/tests/env_guard.sh
set -e
cd "$(dirname "$0")/../.."
# 1. sourcing env.sh clears the shared target dir
( export CARGO_TARGET_DIR=/home/dev/.cache-dev/cargo-target; . experiment/env.sh; [ -z "${CARGO_TARGET_DIR:-}" ] ) || { echo "FAIL 1: env.sh did not clear CARGO_TARGET_DIR"; exit 1; }
# 2. msx_require_env rejects a target dir outside work/
( . experiment/env.sh; export CARGO_TARGET_DIR=/home/dev/.cache-dev/cargo-target; ! msx_require_env 2>/dev/null ) || { echo "FAIL 2: msx_require_env accepted foreign target dir"; exit 1; }
# 3. msx_cargo builds into work/target/<name>
( . experiment/env.sh; msx_cargo guardtest build -q --manifest-path experiment/Cargo.toml -p msx-harness \
    && [ -d work/target/guardtest/debug ] ) || { echo "FAIL 3: msx_cargo build"; exit 1; }
echo "env_guard: OK"
