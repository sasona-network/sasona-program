#!/usr/bin/env bash
# Build and test the program. Run from WSL or Linux.
#
#     bash scripts/build.sh          # build, then test against the built binary
#     bash scripts/build.sh build
#     bash scripts/build.sh test
#
# The build happens in a copy outside this folder, because anchor writes a
# target directory over a gigabyte in size.
#
# anchor build can print an error and still exit 0, so success here means a
# new binary exists and its hash is printed, not that a command returned 0.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="${SASONA_BUILD_DIR:-$HOME/.cache/sasona-program-build}"
KEYPAIR="${SASONA_PROGRAM_KEYPAIR:-$HOME/.config/sasona/program-keypair.json}"
WHAT="${1:-all}"
SO="$WORK/target/deploy/sasona.so"

mkdir -p "$WORK"
rsync -a --delete --exclude target --exclude .git "$REPO/" "$WORK/"
cd "$WORK"

if [ "$WHAT" = build ] || [ "$WHAT" = all ]; then
    # The program's address comes from this keypair. It lives outside the
    # repository and is copied in only for the build.
    mkdir -p target/deploy
    if [ -f "$KEYPAIR" ]; then
        cp "$KEYPAIR" target/deploy/sasona-keypair.json
    fi
    rm -f "$SO"
    anchor build
    [ -f "$SO" ] || { echo "no binary was produced; read the output above"; exit 1; }
    echo "binary  $(stat -c%s "$SO") bytes"
    echo "sha256  $(sha256sum "$SO" | cut -d' ' -f1)"
fi

if [ "$WHAT" = test ] || [ "$WHAT" = all ]; then
    [ -f "$SO" ] || { echo "build first: the tests run against the real binary"; exit 1; }
    SASONA_SO="$SO" cargo test -p sasona -- --test-threads=1
fi
