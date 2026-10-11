#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
case "${1:-test}" in
  build)
    cargo build --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features cli --bin emailops-cli
    ;;
  test)
    python3 -B -m unittest discover -s tools/source_bridge -p 'test_*.py'
    cargo test --manifest-path src-tauri/Cargo.toml --locked --no-default-features --features cli --test source_project
    ;;
  smoke)
    python3 tools/source_bridge/smoke.py --cli src-tauri/target/debug/emailops-cli
    ;;
  *) echo "usage: $0 build|test|smoke" >&2; exit 2 ;;
esac
