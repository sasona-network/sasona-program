#!/usr/bin/env bash
# Break the program on purpose and check that the tests notice.
#
#     bash scripts/mutate.sh
#
# Each mutation is a mistake someone could plausibly make. A copy of the
# program is edited, built and tested. The mutation is "caught" when the
# program still compiles and at least one test fails. A mutation the tests
# miss means a test is missing.
#
# The unmutated program runs first. If its tests do not pass, nothing after
# it would mean anything, so the script stops there.
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="${SASONA_MUTANT_DIR:-$HOME/.cache/sasona-mutant}"
KEYPAIR="${SASONA_PROGRAM_KEYPAIR:-$HOME/.config/sasona/program-keypair.json}"
LIB=programs/sasona/src/lib.rs
SO="$WORK/target/deploy/sasona.so"

# name | perl substitution applied to lib.rs
MUTANTS=(
  "cover vault held by the depositor|s/(COVER_VAULT_SEED\], bump,\s+token::mint = coin_mint, token::authority = )pool/\${1}depositor/"
  "outside forgets the guarantee|s/pool\.outside = add\(free_coins, guarantee_coins\)\?;/pool.outside = free_coins;/"
  "any token accepted as the dollar|s/#\[account\(address = USD_MINT @ SasonaError::NotTheDollar, mint::decimals = USD_DECIMALS\)\]/#[account(mint::decimals = USD_DECIMALS)]/"
  "dollar decimals unchecked|s/#\[account\(address = USD_MINT @ SasonaError::NotTheDollar, mint::decimals = USD_DECIMALS\)\]/#[account(address = USD_MINT @ SasonaError::NotTheDollar)]/"
  "zero deposit allowed|s/require!\(amount > 0, SasonaError::NothingDeposited\);//"
  "depositor's dollars not checked for owner|s/token::mint = usd_mint, token::authority = depositor/token::mint = usd_mint/"
  "fee kept in the pool's reserve|s/(a\.cpi_transfer\(&a\.depositor_usd, )&a\.fees(, a\.depositor\.to_account_info\(\)\), s\.fee)/\${1}&a.pool_usd\${2}/"
  "free coins minted twice|s/(token::mint_to\(a\.cpi_mint\(&a\.depositor_coin\)\.with_signer\(signer\), )free_coins\)/\${1}free_coins * 2)/"
  "a key can freeze the coin|s/mint::decimals = COIN_DECIMALS, mint::authority = pool\)/mint::decimals = COIN_DECIMALS, mint::authority = pool, mint::freeze_authority = depositor)/"
  "deposit: pool side rounded up|s/let into_pool = at_price\(s\.rest, coin_before, usd_before\)\?;/let into_pool = at_price(s.rest, coin_before, usd_before)? + 1;/"
  "deposit: pool side rounded up, price check removed|s/let into_pool = at_price\(s\.rest, coin_before, usd_before\)\?;/let into_pool = at_price(s.rest, coin_before, usd_before)? + 1;/; s/\s*pool\.price_held\(usd_before, coin_before\)\?;//"
  "deposit: dollars not recorded|s/pool\.usd_reserve = add\(usd_before, s\.rest\)\?;/pool.usd_reserve = usd_before;/"
  "deposit: a second deposit replaces the guarantee|s/g\.shares = add\(g\.shares, shares\)\?;/g.shares = shares;/"
  "deposit: any token accepted as the dollar|s/#\[account\(address = pool\.usd_mint @ SasonaError::NotTheDollar\)\]\s*//"
  "deposit: depositor's dollars not checked for owner|s/(pub struct Deposit.*?)token::mint = usd_mint, token::authority = depositor/\${1}token::mint = usd_mint/s"
  "deposit: guarantee minted to the depositor|s/(pub fn deposit.*?a\.cpi_mint\(&a\.)cover_vault/\${1}depositor_coin/s"
  "deposit: fee kept in the pool's reserve|s/(pub fn deposit.*?a\.cpi_transfer\(&a\.depositor_usd, )&a\.fees/\${1}&a.pool_usd/s"
  "deposit: tiny deposits allowed|s/\s*require!\(s\.fee > 0 && free_coins > 0 && guarantee_coins > 0, SasonaError::TooSmall\);//"
  "deposit: deposits too small for a fee allowed|s/require!\(s\.fee > 0 && /require!(/"
  "deposit: pool's coin account not pinned|s/#\[account\(mut, seeds = \[POOL_COIN_SEED\], bump\)\]/#[account(mut)]/"
  "deposit: fee account not pinned|s/#\[account\(mut, seeds = \[FEES_SEED\], bump\)\]/#[account(mut)]/"
  "deposit: pool's dollar account not pinned|s/#\[account\(mut, seeds = \[POOL_USD_SEED\], bump\)\]/#[account(mut)]/"
  "fee: reserve not kept as depth|s/\s*pool\.usd_reserve = add\(pool\.usd_reserve, f\.reserve\)\?;//"
  "fee: the whole markup buys coin|s/f\.buy\(\),\n(\s*)f\.burn,/markup,\n\${1}f.burn,/"
  "fee: nothing burned|s/if burned > 0 \{/if false {/"
  "fee: the participants' coin never sent|s/if shared > 0 \{/if false {/"
  "fee: buying rounded in the buyer's favour|s/let out = coins_out\(dollars, usd, coins\)\?;/let out = coins_out(dollars, usd, coins)? + 1;/"
  "fee: rounded up and the product check removed|s/let out = coins_out\(dollars, usd, coins\)\?;/let out = coins_out(dollars, usd, coins)? + 1;/; s/\s*require!\(\s*pool\.usd_reserve as u128 \* pool\.coin_reserve as u128 >= usd as u128 \* coins as u128,\s*SasonaError::PriceMoved\s*\);//"
  "fee: any token accepted as the dollar|s/(pub struct PayFee.*?)#\[account\(address = pool\.usd_mint @ SasonaError::NotTheDollar\)\]\s*/\${1}/s"
  "fee: payer's dollars not checked for owner|s/token::mint = usd_mint, token::authority = payer/token::mint = usd_mint/"
  "fee: network account not pinned|s/(pub struct PayFee.*?)#\[account\(init_if_needed, payer = payer, seeds = \[NETWORK_SEED\], bump,\s*token::mint = coin_mint, token::authority = pool\)\]/\${1}#[account(mut)]/s"
  "fee: pool's coin account not pinned|s/(pub struct PayFee.*?)#\[account\(mut, seeds = \[POOL_COIN_SEED\], bump\)\]/\${1}#[account(mut)]/s"
  "settle: entry fees not cleared|s/\s*ctx\.accounts\.pool\.fees_held = 0;//"
  "settle: nothing burned|s/let burn = burn_of\(amount\)\?;/let burn = 0;/"
  "fee: burn rounded down|s/\(amount as u128 \* BURN_BPS as u128\)\.div_ceil\(10_000\)/amount as u128 * BURN_BPS as u128 \/ 10_000/"
  "fee: reserve added after the buy|s/(\s*let pool = &mut ctx\.accounts\.pool;\n\s*pool\.usd_reserve = add\(pool\.usd_reserve, f\.reserve\)\?;)(\n\s*let \(burned, shared\) = buy_and_share\((?:.|\n)*?\)\?;)/\${2}\${1}/"
  "fee: burned coins counted as outside|s/pool\.outside = add\(pool\.outside, shared\)\?;/pool.outside = add(pool.outside, out)?;/"
  "depth: dollars not recorded|s/\s*pool\.usd_reserve = add\(pool\.usd_reserve, amount\)\?;//"
  "depth: nothing added allowed|s/(pub fn add_depth.*?)require!\(amount > 0, SasonaError::NothingDeposited\);/\${1}/s"
  "depth: any token accepted as the dollar|s/(pub struct AddDepth.*?)#\[account\(address = pool\.usd_mint @ SasonaError::NotTheDollar\)\]\s*/\${1}/s"
  "depth: giver's dollars not checked for owner|s/token::mint = usd_mint, token::authority = giver/token::mint = usd_mint/"
  "depth: pool's dollar account not pinned|s/(pub struct AddDepth.*?)#\[account\(mut, seeds = \[POOL_USD_SEED\], bump\)\]/\${1}#[account(mut)]/s"
  "claim: anyone can approve|s/\s*require_keys_eq!\(ctx\.accounts\.judge\.key\(\), JUDGE, SasonaError::NotTheJudge\);//"
  "claim: the pool's coins not burned|s/for from in \[a\.pool_coin\.to_account_info\(\), a\.cover_vault\.to_account_info\(\)\]/for from in [a.cover_vault.to_account_info()]/"
  "claim: the cover not burned|s/for from in \[a\.pool_coin\.to_account_info\(\), a\.cover_vault\.to_account_info\(\)\]/for from in [a.pool_coin.to_account_info()]/"
  "claim: the cover's record not reduced|s/\s*cover\.coins -= burn;//"
  "claim: pool side rounded down|s/\(dollars as u128 \* coins as u128\)\.div_ceil\(usd as u128\)/dollars as u128 * coins as u128 \/ usd as u128/"
  "claim: larger than the cover allowed|s/burn < coins && burn < cover\.coins/burn < coins/"
  "claim: paid in any token|s/(pub struct Claim.*?#\[account\(mut, )token::mint = usd_mint,\s*/\${1}/s"
  "claim: cover vault not pinned|s/(pub struct Claim.*?)#\[account\(mut, seeds = \[COVER_VAULT_SEED\], bump\)\]/\${1}#[account(mut)]/s"
  "release request: shares not taken from the guarantee|s/g\.shares = g\.shares\.checked_sub\(shares\)\.ok_or\(SasonaError::MoreThanIsThere\)\?;/g.shares.checked_sub(shares).ok_or(SasonaError::MoreThanIsThere)?;/"
  "release request: no notice|s/\.checked_add\(NOTICE_SECONDS\)/.checked_add(0)/"
  "release: notice not checked|s/\s*require!\(Clock::get\(\)\?\.unix_timestamp >= ctx\.accounts\.exit\.ready_at, SasonaError::NoticeNotOver\);//"
  "release: cover shares not reduced|s/\s*cover\.shares -= shares;//"
  "deposit: an old guarantee mixed with shares|s/\s*require!\(ctx\.accounts\.legacy_vault\.data_is_empty\(\), SasonaError::JoinCoverFirst\);\n(\s*let s = Slices)/\n\${1}/"
  "join: rent to anyone|s/#\[account\(mut, address = guarantee\.owner\)\]/#[account(mut)]/"
  "join: record not converted to shares|s/\s*ctx\.accounts\.guarantee\.shares = shares;//"
  "claim: the cover may get too thin|s/\s*require!\(\s*\(cover\.coins - burn\) as u128 \* MAX_SHARES_PER_COIN as u128 >= cover\.shares as u128,\s*SasonaError::CoverTooThin\s*\);//"
  "claim: paid into the fee account|s/,\s*constraint = claimant_usd\.key\(\) != fees\.key\(\) @ SasonaError::NotTheClaimant//"
  "release request: an old guarantee mixed with shares|s/(pub fn request_release.*?)require!\(ctx\.accounts\.legacy_vault\.data_is_empty\(\), SasonaError::JoinCoverFirst\);/\${1}/s"
  "join: adds to the record instead of replacing it|s/ctx\.accounts\.guarantee\.shares = shares;/ctx.accounts.guarantee.shares += shares;/"
  "cover: new shares rounded up|s/let v = coins as u128 \* cover_shares as u128 \/ cover_coins as u128;/let v = (coins as u128 * cover_shares as u128).div_ceil(cover_coins as u128);/"
  "fee: coin mint not pinned|s/(pub struct PayFee.*?)#\[account\(mut, address = pool\.coin_mint\)\]/\${1}#[account(mut)]/s"
  "settle: fee account not pinned|s/(pub struct SettleEntryFees.*?)#\[account\(mut, seeds = \[FEES_SEED\], bump\)\]/\${1}#[account(mut)]/s"
  "settle: network account not pinned|s/(pub struct SettleEntryFees.*?)#\[account\(init_if_needed, payer = caller, seeds = \[NETWORK_SEED\], bump,\s*token::mint = coin_mint, token::authority = pool\)\]/\${1}#[account(mut)]/s"
)

# Fresh copy of the source, keeping target/ so each build is incremental.
reset() {
    mkdir -p "$WORK"
    rsync -a --delete --exclude target --exclude .git "$REPO/" "$WORK/"
    mkdir -p "$WORK/target/deploy"
    cp "$KEYPAIR" "$WORK/target/deploy/sasona-keypair.json"
}

build() { (cd "$WORK" && rm -f "$SO" && anchor build >/dev/null 2>&1) && [ -f "$SO" ]; }
tests_compile() { (cd "$WORK" && cargo test -q -p sasona --no-run >/dev/null 2>&1); }
tests_pass() { (cd "$WORK" && SASONA_SO="$SO" cargo test -q -p sasona -- --test-threads=1 >/dev/null 2>&1); }

reset
if ! build || ! tests_compile || ! tests_pass; then
    echo "the unmutated program does not build and pass; fix that first"
    exit 2
fi
echo "baseline passes"

missed=0
for m in "${MUTANTS[@]}"; do
    name="${m%%|*}"; sub="${m#*|}"
    # SASONA_ONLY=regex runs only the mutations whose names match, for a step
    # that added code without touching what earlier steps already tested.
    if [ -n "${SASONA_ONLY:-}" ] && ! [[ "$name" =~ $SASONA_ONLY ]]; then continue; fi
    reset
    before=$(sha256sum "$WORK/$LIB")
    perl -0pi -e "$sub" "$WORK/$LIB"
    if [ "$(sha256sum "$WORK/$LIB")" = "$before" ]; then
        echo "BROKEN   $name: the edit matched nothing, so it tested nothing"
        missed=1; continue
    fi
    if ! build || ! tests_compile; then
        echo "caught   $name (does not compile)"; continue
    fi
    if tests_pass; then
        echo "MISSED   $name"; missed=1
    else
        echo "caught   $name"
    fi
done
exit $missed
