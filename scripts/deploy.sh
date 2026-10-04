#!/usr/bin/env bash
# Upgrade the program on devnet from the binary scripts/build.sh made.
#
#     bash scripts/deploy.sh
#
# The public devnet endpoint limits how fast it accepts transactions, and a
# plain `solana program deploy` of a 600 KB program can stall for hours. So
# the binary is written into a buffer whose key we hold, and every failed
# attempt resumes where the last one stopped. The program is upgraded from
# the buffer only once all of it matches the binary, and the bytes on chain
# are compared with the build at the end.
set -u

KEYPAIR="${SASONA_DEPLOYER:-$HOME/.config/sasona/devnet-deployer.json}"
BUFFER_KEY="${SASONA_BUFFER_KEY:-$HOME/.config/sasona/deploy-buffer.json}"
PROGRAM=7eiHSnDkM4WjJdY36D2Yqsjw893mCMUtBAwMCQ5adL99
SO="${SASONA_BUILD_DIR:-$HOME/.cache/sasona-program-build}/target/deploy/sasona.so"
URL=devnet

[ -f "$SO" ] || { echo "no binary at $SO; run scripts/build.sh first"; exit 2; }
[ -f "$BUFFER_KEY" ] || solana-keygen new --no-bip39-passphrase -s -o "$BUFFER_KEY" >/dev/null
BUFFER=$(solana-keygen pubkey "$BUFFER_KEY")
SIZE=$(stat -c %s "$SO")
echo "binary  $SIZE bytes, sha256 $(sha256sum "$SO" | cut -c1-16)"

# A buffer left by a run for a binary of another size would keep that
# binary's tail, and its last chunk would never match. Close it, getting its
# rent back, and start a new one.
BLEN=$(solana program show -u "$URL" "$BUFFER" 2>/dev/null | awk '/Data Length/ {print $3}')
if [ -n "$BLEN" ] && [ "$BLEN" != "$SIZE" ]; then
    echo "buffer holds $BLEN bytes, not $SIZE; closing it"
    solana program close -u "$URL" --keypair "$KEYPAIR" --authority "$KEYPAIR" "$BUFFER" || exit 1
fi

# The program account must be large enough for the new binary.
HAVE=$(solana program show -u "$URL" "$PROGRAM" | awk '/Data Length/ {print $3}')
if [ "$HAVE" -lt "$SIZE" ]; then
    solana program extend -u "$URL" --keypair "$KEYPAIR" "$PROGRAM" $((SIZE - HAVE)) || exit 1
fi

# Chunks of the buffer that do not yet match the binary.
unwritten() {
    solana account -u "$URL" "$BUFFER" --output-file /tmp/sasona-buffer.bin >/dev/null 2>&1 || { echo "?"; return; }
    python3 -c "
so = open('$SO', 'rb').read()
b = open('/tmp/sasona-buffer.bin', 'rb').read()[37:]
print(sum(1 for i in range(0, len(so), 1000) if b[i:i + 1000] != so[i:i + 1000]))"
}

left="?"
for try in $(seq 1 40); do
    solana program write-buffer -u "$URL" --keypair "$KEYPAIR" --buffer "$BUFFER_KEY" --buffer-authority "$KEYPAIR" \
        --max-sign-attempts 100 --with-compute-unit-price 20000 "$SO" >/dev/null 2>&1
    left=$(unwritten)
    echo "try $try: $left of $(( (SIZE + 999) / 1000 )) chunks still to write"
    [ "$left" = 0 ] && break
    sleep 20
done
[ "$left" = 0 ] || { echo "the buffer is not complete; run again to resume"; exit 1; }

solana program upgrade -u "$URL" --keypair "$KEYPAIR" --upgrade-authority "$KEYPAIR" "$BUFFER" "$PROGRAM" || exit 1
solana program dump -u "$URL" "$PROGRAM" /tmp/sasona-onchain.so >/dev/null
ON=$(head -c "$SIZE" /tmp/sasona-onchain.so | sha256sum | cut -d' ' -f1)
PAD=$(tail -c +$((SIZE + 1)) /tmp/sasona-onchain.so | tr -d '\0' | wc -c)
BUILT=$(sha256sum "$SO" | cut -d' ' -f1)
if [ "$ON" = "$BUILT" ] && [ "$PAD" = 0 ]; then
    echo "on chain: $ON, the same bytes as the build"
else
    echo "on chain: $ON, NOT the build ($BUILT)"; exit 1
fi
