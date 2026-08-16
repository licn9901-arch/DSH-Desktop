#!/bin/zsh
set -euo pipefail
cd "${0:A:h}"
command -v node >/dev/null || { print -u2 'Node.js is required.'; exit 1; }
command -v npm >/dev/null || { print -u2 'npm is required.'; exit 1; }
command -v cargo >/dev/null || { print -u2 'Rust is required: https://rustup.rs'; exit 1; }
npm run build:macos
