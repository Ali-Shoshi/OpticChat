#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
export PATH="${HOME}/.cargo/bin:${PATH}"
if [[ ! -x target/release/optic-chat ]]; then
  ./scripts/build.sh build
fi
exec ./target/release/optic-chat "$@"
