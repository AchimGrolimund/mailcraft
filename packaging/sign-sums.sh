#!/usr/bin/env bash
# Sign a release's SHA256SUMS.txt with MailCraft's own release key: writes SHA256SUMS.txt.sig
# beside it (an ed25519 signature in base64, docs/releasing.md). A copy of MailCraft installs an
# update only when this signature verifies by one of the public keys it carries.
#
#   MAILCRAFT_RELEASE_KEY=… packaging/sign-sums.sh <folder with SHA256SUMS.txt>
#
# The key (base64 PKCS#8, from `cargo xtask release-key new`) is a secret of the protected
# `release` environment and comes in through the environment only; it is never printed. Without
# it the release is unsigned, said as such, and the build carries on (forks, dry runs).
set -euo pipefail

DIR="${1:?usage: packaging/sign-sums.sh <folder with SHA256SUMS.txt>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SUMS="$(cd "$DIR" && pwd)/SHA256SUMS.txt"

if [ ! -f "$SUMS" ]; then
  echo "::error::$SUMS isn't there"
  exit 1
fi
rm -f "$SUMS.sig"
if [ -z "${MAILCRAFT_RELEASE_KEY:-}" ]; then
  echo "unsigned: MAILCRAFT_RELEASE_KEY isn't set, so there is no SHA256SUMS.txt.sig and no copy of MailCraft will install this release by itself"
  exit 0
fi
cd "$ROOT"
cargo run --locked --quiet --package xtask -- release-key sign "$SUMS"
