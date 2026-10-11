#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
stage="$(mktemp -d)"
trap 'rm -rf -- "$stage"' EXIT
cp src-tauri/target/debug/emailops-cli "$stage/"
mkdir "$stage/source_bridge"
cp tools/source_bridge/gateway.py tools/source_bridge/n8n-project.json tools/source_bridge/README.md "$stage/source_bridge/"
mkdir -p release
tar -czf release/emailops-cli-linux-x86_64-prototype.tar.gz -C "$stage" .
