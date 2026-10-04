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
  "round: seed not checked|s/\s*require!\(solana_sha256_hasher::hashv\(&\[&seed\]\)\.to_bytes\(\) == r\.seed_hash, SasonaError::WrongSeed\);//"
  "round: no delay before the entropy slot|s/let target = r\.commit_slot \+ ENTROPY_DELAY_SLOTS;/let target = r.commit_slot;/"
  "round: the latest slot hash instead of the earliest|s/if e\.0 < target \{\s*break;\s*\}\s*found = e;/let _ = e;/"
  "round: bond not returned|s/\*\*round_info\.try_borrow_mut_lamports\(\)\? -= ROUND_BOND_LAMPORTS;\s*\*\*ctx\.accounts\.opener\.to_account_info\(\)\.try_borrow_mut_lamports\(\)\? \+= ROUND_BOND_LAMPORTS;//"
  "round: no bond taken|s/anchor_lang::system_program::transfer\((?:.|\n)*?ROUND_BOND_LAMPORTS,\n\s*\)\?;//"
  "round: more picks than candidates allowed|s/ && count as u32 <= pool_size//"
  "round: no limit on candidates|s/pool_size <= MAX_ROUND_CANDIDATES && //"
  "round: marked withheld while it can still be revealed|s/\s*require!\(matches!\(entropy_for\(&data, target\)\?, Entropy::Gone\), SasonaError::NotWithheldYet\);//"
  "round: revealed twice|s/(pub fn reveal_round.*?)require!\(r\.state == ROUND_COMMITTED, SasonaError::RoundNotOpen\);/\${1}/s"
  "round: no domain in the final seed|s/hashv\(&\[DRAW_DOMAIN, &seed, &entropy\]\)/hashv(\&[\&seed, \&entropy])/"
  "round: slot hashes not pinned|s/(pub struct RevealRound.*?)#\[account\(address = SLOT_HASHES_ID\)\]/\${1}/s"
  "round: bond paid to anyone|s/#\[account\(mut, address = round\.opener\)\]/#[account(mut)]/"
  "round: the oldest entry treated as already gone|s/if oldest > target \{/if oldest >= target {/"
  "round: one list drawn many times|s/seeds = \[ROUND_SEED, pool_fingerprint\.as_ref\(\)\], bump\)\]/seeds = [ROUND_SEED, pool_fingerprint.as_ref(), opener.key().as_ref()], bump)]/"
  "reading: any question revealed|s/\s*require!\(solana_sha256_hasher::hashv\(&\[&question\]\)\.to_bytes\(\) == r\.question_hash, SasonaError::WrongQuestion\);//"
  "reading: a nonce reused across readings|s/seeds = \[NONCE_SEED, nonce\.as_ref\(\)\], bump\)\]/seeds = [NONCE_SEED, nonce.as_ref(), reading.key().as_ref()], bump)]/"
  "reading: revealed in the slot it was committed|s/require!\(now > r\.commit_slot, SasonaError::TooEarly\);/require!(now >= r.commit_slot, SasonaError::TooEarly);/"
  "reading: revealed after its window|s/\s*require!\(now <= r\.commit_slot \+ REVEAL_WINDOW_SLOTS, SasonaError::TooLate\);//"
  "reading: any verdict number|s/\s*require!\(\(1\.\.=3\)\.contains\(&verdict\), SasonaError::BadVerdict\);//"
  "reading: lapsed while it can still be revealed|s/\s*require!\(Clock::get\(\)\?\.slot > r\.commit_slot \+ REVEAL_WINDOW_SLOTS, SasonaError::NotLapsedYet\);//"
  "reading: revealed by anyone|s/#\[account\(mut, has_one = reader @ SasonaError::NotTheReader\)\]/#[account(mut)]/"
  "reading: a round not yet drawn|s/\s*require!\(round\.state == ROUND_DRAWN, SasonaError::RoundNotDrawn\);//"
  "reading: the service's hash unchecked|s/\s*require!\(solana_sha256_hasher::hashv\(&\[endpoint\.as_bytes\(\)\]\)\.to_bytes\(\) == endpoint_hash, SasonaError::BadEndpoint\);//"
  "reading: revealed twice|s/(pub fn reveal_reading.*?)require!\(r\.state == READING_COMMITTED, SasonaError::ReadingNotOpen\);/\${1}/s"
  "reading: the question built with another tier|s/tier\\\\\":1/tier\\\\\":2/"
  "reading: a later commitment takes the nonce|s/\s*require!\(committed < used\.commit_slot, SasonaError::NonceTaken\);//"
  "reading: a commitment in the same slot takes the nonce|s/require!\(committed < used\.commit_slot, SasonaError::NonceTaken\);/require!(committed <= used.commit_slot, SasonaError::NonceTaken);/"
  "reading: the first reveal keeps the nonce for good|s/require!\(committed < used\.commit_slot, SasonaError::NonceTaken\);/require!(false, SasonaError::NonceTaken);/"
  "pair: an unrevealed reading re-tested|s/\s*require!\(first\.state == READING_REVEALED, SasonaError::FirstNotRevealed\);//"
  "pair: another service accepted|s/\s*require!\(first\.endpoint == endpoint, SasonaError::NotTheSameService\);//"
  "pair: the first reader checks themselves|s/\s*require!\(first\.reader != ctx\.accounts\.reader\.key\(\), SasonaError::SameReader\);//"
  "pair: re-read round committed in the slot the first was revealed|s/require!\(first\.reveal_slot < ctx\.accounts\.round\.commit_slot, SasonaError::TooEarly\);/require!(first.reveal_slot <= ctx.accounts.round.commit_slot, SasonaError::TooEarly);/"
  "pair: re-read round committed before the first was revealed|s/require!\(first\.reveal_slot < ctx\.accounts\.round\.commit_slot, SasonaError::TooEarly\);/require!(first.reveal_slot < Clock::get()?.slot, SasonaError::TooEarly);/"
  "pair: settled twice|s/\s*require!\(p\.outcome == 0, SasonaError::AlreadySettled\);//"
  "pair: settled before the second is revealed|s/\s*require!\(second\.state == READING_REVEALED, SasonaError::ReadingNotOpen\);//"
  "pair: settled against any first reading|s/has_one = first, has_one = second/has_one = second/"
  "pair: a failing first and a delivering second read as decay|s/\(\_, 1\) => PAIR_WORKS_NOW,\s*\(1, _\) => PAIR_FALSE_OR_DECAYED,/(1, _) => PAIR_FALSE_OR_DECAYED, (_, 1) => PAIR_WORKS_NOW,/"
  "member: joining locks nothing|s/(authority: a\.owner\.to_account_info\(\),\s*\},\s*\),\s*)MEMBER_STAKE,/\${1}0,/s"
  "member: the roster not lengthened|s/\s*members\.seated = members\.seated\.checked_add\(1\)\.ok_or\(SasonaError::Overflow\)\?;//"
  "member: the membership not told its seat|s/m\.seat = seat;/m.seat = 0;/"
  "member: asking to leave keeps the seat|s/(require!\(ctx\.accounts\.member\.state == MEMBER_ACTIVE, SasonaError::NotActive\);\n)\s*unseat\([^\n]*\n/\$1/"
  "member: asking to leave keeps it active|s/m\.state = MEMBER_LEAVING;/m.state = MEMBER_ACTIVE;/"
  "member: leaves without notice|s/\s*require!\(Clock::get\(\)\?\.unix_timestamp >= m\.leave_at, SasonaError::NoticeNotOver\);//"
  "member: leaves with a challenge open|s/\s*require!\(m\.open_challenges == 0, SasonaError::ChallengeOpen\);//"
  "member: leaves without asking|s/\s*require!\(m\.state == MEMBER_LEAVING, SasonaError::NotLeaving\);//"
  "member: anyone asks to leave|s/(pub struct AskToLeave.*?),\s*has_one = owner @ SasonaError::NotTheOwner/\$1/s"
  "member: anyone takes the stake back|s/(pub struct Leave<.*?),\s*has_one = owner @ SasonaError::NotTheOwner/\$1/s"
  "member: the challenge count not raised|s/m\.open_challenges = m\.open_challenges\.checked_add\(1\)\.ok_or\(SasonaError::Overflow\)\?;//"
  "roster: the last seat not moved up|s/seat\.member = last_seat\.member;/seat.member = 0;/"
  "roster: the membership moved not told|s/\s*mover\.seat = k;//"
  "roster: the last seat not emptied|s/\s*last_seat\.member = 0;//"
  "roster: not shortened|s/members\.seated = last - 1;/members.seated = last;/"
  "roster: any membership moves up|s/\s*require!\(mover\.number == last_seat\.member && mover\.seat == last, SasonaError::NotTheSeat\);//"
  "roster: any account stands in for a seat|s/\s*require_keys_eq!\(info\.key\(\), at, SasonaError::NotTheSeat\);//"
  "roster: a round counts every membership ever taken|s/r\.members = ctx\.accounts\.members\.seated;/r.members = ctx.accounts.members.count;/"
  "draw: any seated member reads|s/if k == member\.seat \{/if true {/"
  "draw: a seat passed over unchecked|s/\s*require!\(s\.since >= round\.commit_slot \|\| Some\(s\.owner\) == first_reader, SasonaError::NotSkippable\);//"
  "draw: a seat sat in since cannot be passed over|s/require!\(s\.since >= round\.commit_slot \|\| Some\(s\.owner\) == first_reader,/require!(Some(s.owner) == first_reader,/"
  "draw: a reader who sat down since reads|s/\s*require!\(seat\.since < round\.commit_slot, SasonaError::SatDownSince\);//"
  "draw: sitting down in the round.s slot counts|s/require!\(seat\.since < round\.commit_slot,/require!(seat.since <= round.commit_slot,/"
  "member: the slot it sat down in not recorded|s/s\.since = Clock::get\(\)\?\.slot;/s.since = 0;/"
  "roster: moving up keeps the old slot|s/\s*seat\.since = Clock::get\(\)\?\.slot;//"
  "draw: seats beyond the roster drawn|s/\s*if k > members\.seated \{\s*continue;\s*\}//"
  "draw: seats shown that nobody passed over|s/\s*require!\(shown == skipped\.len\(\), SasonaError::NotDrawn\);//"
  "draw: someone else.s membership|s/\s*require_keys_eq!\(member\.owner, reader, SasonaError::NotTheReader\);//"
  "draw: another label|s/b.reader., endpoint_hash/b\x22readers\x22, endpoint_hash/"
  "draw: the attempt little-endian|s/&attempt\.to_be_bytes\(\)\]/&attempt.to_le_bytes()]/"
  "draw: no window to read|s/\s*require!\(Clock::get\(\)\?\.slot <= closes, SasonaError::WindowClosed\);//"
  "draw: the window a slot too long|s/require!\(Clock::get\(\)\?\.slot <= closes,/require!(Clock::get()?.slot <= closes + 1,/"
  "reading: the membership not recorded|s/(start_reading\(&a\.round[^\n]*\)\?;\n)\s*a\.reading\.member = a\.member\.number;\n(\s*Ok\(\(\)\))/\$1\$2/"
  "reading: the reveal time not recorded|s/\s*r\.reveal_time = Clock::get\(\)\?\.unix_timestamp;//"
  "pair: a first reading upheld false still settles|s/\s*require!\(ctx\.accounts\.first\.state == READING_REVEALED, SasonaError::FirstNotRevealed\);//"
  "challenge: an unrevealed reading|s/\s*require!\(r\.state == READING_REVEALED, SasonaError::ReadingNotOpen\);//"
  "challenge: no bond|s/(to: ctx\.accounts\.challenge\.to_account_info\(\),\s*\},\s*\),\s*)CHALLENGE_BOND_LAMPORTS/\${1}0/s"
  "challenge: at any time|s/\s*require!\(now <= r\.reveal_time\.checked_add\(CHALLENGE_WINDOW_SECONDS\)\.ok_or\(SasonaError::Overflow\)\?, SasonaError::WindowClosed\);//"
  "challenge: not on the thirtieth day|s/require!\(now <= r\.reveal_time\.checked_add/require!(now < r.reveal_time.checked_add/"
  "answer: any reply|s/\s*require!\(solana_sha256_hasher::hashv\(&\[reply\]\)\.to_bytes\(\) == r\.reply_hash, SasonaError::NotTheReply\);//"
  "answer: any verdict|s/\s*require!\(verdict_of\(reply, &nonce\) == r\.verdict, SasonaError::VerdictDoesNotFollow\);//"
  "answer: any nonce|s/\s*require_keys_eq!\(ctx\.accounts\.used_nonce\.reading, r\.key\(\), SasonaError::NonceTaken\);//"
  "answer: after the deadline|s/\s*require!\(Clock::get\(\)\?\.unix_timestamp <= c\.deadline, SasonaError::WindowClosed\);//"
  "answer: answered twice|s/(pub fn answer_challenge.*?)\s*require!\(c\.state == CHALLENGE_OPEN, SasonaError::ChallengeClosed\);/\$1/s"
  "answer: the reply not sealed|s/\s*ctx\.accounts\.evidence\.sealed = true;//"
  "evidence: written after it was sealed|s/\s*require!\(!e\.sealed, SasonaError::EvidenceSealed\);//"
  "evidence: anyone writes it|s/(pub struct WriteEvidence.*?)#\[account\(has_one = reader @ SasonaError::NotTheReader\)\]/\${1}#[account()]/s"
  "evidence: written past its end|s/\s*require!\(end <= e\.reply\.len\(\), SasonaError::ReplyTooLong\);//"
  "evidence: no size limit|s/\s*require!\(len <= MAX_REPLY_BYTES, SasonaError::ReplyTooLong\);//"
  "quote: anyone sets it|s/(pub struct SetQuote.*?)#\[account\(has_one = reader @ SasonaError::NotTheReader\)\]/\${1}#[account()]/s"
  "quote: above 10,000 basis points|s/\s*require!\(rate <= MAX_QUOTE_BPS, SasonaError::BadRate\);//"
  "quote: a service that did not deliver|s/\s*require!\(r\.verdict == 1, SasonaError::NotDelivered\);//"
  "quote: an unrevealed reading|s/(pub fn set_quote.*?)\s*require!\(r\.state == READING_REVEALED, SasonaError::ReadingNotOpen\);/\$1/s"
  "quote: after the reading stops being current|s/\s*require!\(clock\.unix_timestamp <= ends, SasonaError::WindowClosed\);//"
  "quote: by a member who asked to leave|s/(pub fn set_quote.*?)\s*require!\(ctx\.accounts\.member\.state == MEMBER_ACTIVE, SasonaError::NotActive\);/\$1/s"
  "quote: the rate not recorded|s/q\.rate = rate;/q.rate = 0;/"
  "quote: the time not recorded|s/q\.set_time = clock\.unix_timestamp;/q.set_time = 0;/"
  "quote: the membership not recorded|s/q\.member = r\.member;/q.member = 0;/"
  "quote: the lowest rate skips the checks|s/        if rate > 0 \{/        if rate > 1 {/"
  "quote: any membership checked|s/(pub struct SetQuote.*?)#\[account\(seeds = \[MEMBER_SEED, reading\.member\.to_le_bytes\(\)\.as_ref\(\)\], bump = member\.bump\)\]/\${1}#[account()]/s"
  "quote: the slot not recorded|s/q\.set_slot = clock\.slot;/q.set_slot = 0;/"
  "uphold: at the deadline|s/require!\(Clock::get\(\)\?\.unix_timestamp > c\.deadline, SasonaError::NotLapsedYet\);/require!(Clock::get()?.unix_timestamp >= c.deadline, SasonaError::NotLapsedYet);/"
  "uphold: upheld twice|s/(pub fn uphold_challenge.*?)\s*require!\(c\.state == CHALLENGE_OPEN, SasonaError::ChallengeClosed\);/\$1/s"
  "uphold: the reading keeps counting|s/\s*ctx\.accounts\.reading\.state = READING_FALSE;//"
  "uphold: the challenger takes a fifth|s/let reward = stake \/ 10;/let reward = stake \/ 5;/"
  "uphold: the membership is not marked|s/\s*m\.state = MEMBER_SLASHED;//"
  "uphold: a member who asked to leave keeps the stake|s/if state == MEMBER_ACTIVE \|\| state == MEMBER_LEAVING \{/if state == MEMBER_ACTIVE {/"
  "uphold: takes a stake already gone|s/let stake = if state == MEMBER_ACTIVE \|\| state == MEMBER_LEAVING \{ ctx\.accounts\.member\.stake \} else \{ 0 \};/let stake = MEMBER_STAKE;/"
  "uphold: the seat kept|s/\s*if state == MEMBER_ACTIVE \{\s*unseat\([^\n]*\n\s*\}//"
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
tests_pass() { (cd "$WORK" && SASONA_SO="$SO" cargo test -q -p sasona >/dev/null 2>&1); }

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
