#!/usr/bin/env bash
# Build the wasm core and vendor the P2P signaling library into web/.
set -euo pipefail
cd "$(dirname "$0")"
(cd core && cargo build --release --target wasm32-unknown-unknown)
cp core/target/wasm32-unknown-unknown/release/cloverleaf.wasm web/core.wasm
if [ -d node_modules/trystero ]; then
  bun build node_modules/trystero/dist/index.mjs --target=browser --format=esm --minify --outfile web/vendor/trystero-nostr.js
  bun build node_modules/@trystero-p2p/ws-relay/dist/index.mjs --target=browser --format=esm --minify --outfile web/vendor/trystero-ws.js
fi
ls -l web/core.wasm web/vendor/
