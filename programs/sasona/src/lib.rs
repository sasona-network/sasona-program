//! The Sasona pool.
//!
//! The pool holds dollars on one side and Sasona coin on the other. The coin
//! is created by this program, its only mint authority is the pool's own
//! address, and that address has no private key. So the one way a coin can
//! come into existence is through the rules below.
//!
//! While on devnet the program itself is upgradeable, and whoever holds the
//! upgrade authority could deploy different rules. That is the one key that
//! still matters, and it is named in every proof.
//!
//! What every instruction must leave true:
//!
//!   * the coin's supply is exactly what sits in the pool plus what sits
//!     outside it, read from the mint itself rather than from our own count
//!   * the pool holds at least the dollars it has recorded. Anyone can send
//!     tokens to an account, so more is allowed; less never is
//!   * a guarantee can only leave its vault through this program

use anchor_lang::prelude::*;
use anchor_lang::pubkey;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Burn, CloseAccount, Mint, MintTo, Token, TokenAccount, Transfer};

declare_id!("7eiHSnDkM4WjJdY36D2Yqsjw893mCMUtBAwMCQ5adL99");

/// The only token this pool accepts as a dollar. On devnet it is a stand-in
/// we created; mainnet will use a real stablecoin. Fixed in the program so
/// that nobody can open the pool first with a token of their own making.
pub const USD_MINT: Pubkey = pubkey!("ACPdQtaC6HKgT8V4GVRke57vtZrbv3zDqTwvENBoRPCy");
pub const USD_DECIMALS: u8 = 6;

/// The opening price, $0.0002 a coin. Fixed for the same reason: the first
/// deposit sets the ratio every later deposit is measured against.
pub const OPENING_COINS_PER_USD: u64 = 5_000;

pub const POOL_SEED: &[u8] = b"pool";
pub const COIN_SEED: &[u8] = b"coin";
pub const POOL_USD_SEED: &[u8] = b"pool-usd";
pub const POOL_COIN_SEED: &[u8] = b"pool-coin";
pub const FEES_SEED: &[u8] = b"fees";
pub const GUARANTEE_SEED: &[u8] = b"guarantee";
pub const VAULT_SEED: &[u8] = b"guarantee-vault";

/// Same as the dollar it is priced against, so one unit of coin and one unit
/// of dollar are the same size and a price is a plain ratio.
pub const COIN_DECIMALS: u8 = 6;

/// The entry fee comes off the top of a deposit. The other three split what
/// is left, in basis points.
pub const FEE_BPS: u64 = 1_500;
pub const SPREAD_BPS: u64 = 1_500;
pub const GUARANTEE_BPS: u64 = 7_500;
pub const FREE_BPS: u64 = 1_000;

const _: () = assert!(SPREAD_BPS + GUARANTEE_BPS + FREE_BPS == 10_000);
const _: () = assert!(FEE_BPS < 10_000);

/// A purchase carries a markup of fifteen points over the seller's price, all
/// of it paid in dollars. Five of the fifteen are the reserve: they stay in
/// the pool as depth and nothing is minted against them. Three percent of the
/// fifteen is burned. The rest buys coin from the pool for the participants.
pub const MARKUP_POINTS: u64 = 15;
pub const RESERVE_POINTS: u64 = 5;
pub const BURN_BPS: u64 = 300;

/// How the participants' coin is shared, in points.
///
/// NOTE, temporary: none of these roles exists on chain yet. A share whose
/// role nobody filled goes to the network, so for now the network receives
/// all of it, and the network's coin waits in a vault that has no way out.
/// Both change when the parts that bring developers, miners, submitters and
/// marketers on chain are built.
pub const DEVELOPER_POINTS: u64 = 4;
pub const MINERS_POINTS: u64 = 2;
pub const SUBMITTER_POINTS: u64 = 1;
pub const MARKETER_POINTS: u64 = 1;
pub const NETWORK_POINTS: u64 = 2;

const _: () = assert!(
    RESERVE_POINTS + DEVELOPER_POINTS + MINERS_POINTS + SUBMITTER_POINTS + MARKETER_POINTS + NETWORK_POINTS
        == MARKUP_POINTS
);

pub const NETWORK_SEED: &[u8] = b"network";

/// Every depositor's guarantee is cover for every buyer, so the guarantees
/// sit together in one vault and each depositor holds shares of it. A claim
/// reduces the coins behind every share at once.
pub const COVER_SEED: &[u8] = b"cover";
pub const COVER_VAULT_SEED: &[u8] = b"cover-vault";
pub const EXIT_SEED: &[u8] = b"exit";

/// How long a guarantee stays exposed to claims after its owner asks for it
/// back. It has to outlast the window in which a bad purchase can be
/// reported and the time it takes to judge it. The design puts the floor at
/// 31 days and says 45 leaves margin.
///
/// Not yet enough on its own: once the notice has run out, an owner can
/// still release just ahead of a claim they can see coming. Releases will
/// pause while a claim is pending, once claims are filed on chain (part 7).
pub const NOTICE_SECONDS: i64 = 45 * 24 * 60 * 60;

/// A claim may not leave the cover thinner than one coin unit behind every
/// thousand shares. Below that, new deposits would buy so many shares that
/// the count overflows and nobody could deposit again.
pub const MAX_SHARES_PER_COIN: u64 = 1_000;

/// Rounds draw which services get tested; sasona-protocol SPEC.md section 1
/// is the rule. The program holds the commitment, waits for a slot that did
/// not exist when the round was committed, and fixes the final seed.
pub const ROUND_SEED: &[u8] = b"round";
pub const DRAW_DOMAIN: &[u8] = b"sasona/draw/v1";
/// The entropy comes from a slot at least this far after the commitment.
pub const ENTROPY_DELAY_SLOTS: u64 = 32;
pub const SLOT_HASHES_ID: Pubkey = pubkey!("SysvarS1otHashes111111111111111111111111111");

pub const ROUND_COMMITTED: u8 = 0;
pub const ROUND_DRAWN: u8 = 1;
pub const ROUND_WITHHELD: u8 = 2;

/// The most candidates a round may hold, so the draw's attempt counter fits
/// in four bytes.
pub const MAX_ROUND_CANDIDATES: u32 = 65_536;

/// Put up when a round is opened and returned when its seed is revealed.
/// Lost if the seed is withheld, so walking away from a draw you dislike is
/// not free. 0.1 SOL.
pub const ROUND_BOND_LAMPORTS: u64 = 100_000_000;

/// Readings of drawn services; sasona-protocol SPEC.md section 2 is the rule.
/// The question's hash is committed before the service is called and the
/// question revealed after.
pub const READING_SEED: &[u8] = b"reading";
/// About an hour. A reading not revealed by then can only be marked lapsed.
pub const REVEAL_WINDOW_SLOTS: u64 = 9_000;
pub const MAX_ENDPOINT_LEN: usize = 256;
pub const NONCE_SEED: &[u8] = b"nonce";

pub const READING_COMMITTED: u8 = 0;
pub const READING_REVEALED: u8 = 1;
pub const READING_LAPSED: u8 = 2;

/// A second reading's link to the first: sasona-protocol SPEC.md section 3.
pub const PAIR_SEED: &[u8] = b"pair";
pub const PAIR_WORKS_NOW: u8 = 1;
pub const PAIR_FALSE_OR_DECAYED: u8 = 2;
pub const PAIR_AGREED_FAILS: u8 = 3;

/// Members (sasona-protocol SPEC.md section 4). A membership is one stake of
/// MEMBER_STAKE coins, numbered from 1 in the order taken.
pub const MEMBERS_SEED: &[u8] = b"members";
pub const MEMBER_SEED: &[u8] = b"member";
pub const SEAT_SEED: &[u8] = b"seat";
pub const STAKES_SEED: &[u8] = b"stakes";
/// NOTE, temporary: one flat stake, 10,000 coins, about two devnet dollars.
/// A stake that deters has to grow with the traffic a service carries, which
/// needs purchases on chain (parts 7 and 8). This number will change.
pub const MEMBER_STAKE: u64 = 10_000 * 1_000_000;
pub const MEMBER_ACTIVE: u8 = 0;
pub const MEMBER_LEAVING: u8 = 1;
pub const MEMBER_LEFT: u8 = 2;
pub const MEMBER_SLASHED: u8 = 3;
/// SPEC.md 4.3: how many seats are tried before a service goes unread.
pub const MAX_READER_ATTEMPTS: u32 = 16;
/// SPEC.md 4.3: a reading is committed within this many slots of the slot
/// the round's entropy came from, about an hour.
pub const READ_WINDOW_SLOTS: u64 = 9_000;

/// Challenges (SPEC.md section 5).
pub const CHALLENGE_SEED: &[u8] = b"challenge";
pub const EVIDENCE_SEED: &[u8] = b"evidence";
/// Taken stakes wait here, minus the challenger's tenth.
/// NOTE, temporary: nothing can take them out until chargebacks are on chain
/// (part 7), which decides where they go.
pub const HELD_SEED: &[u8] = b"held";
/// NOTE, temporary: the bond and the challenger's tenth are devnet figures,
/// set with the stake from what readings are worth once purchases are on
/// chain (parts 7 and 8).
pub const CHALLENGE_BOND_LAMPORTS: u64 = 100_000_000;
/// SPEC.md 5.1: a reading can be challenged for 30 days after its reveal, the
/// term a reading is current for, and a challenge answered for 7 days. Both
/// end inside the 45 days' notice a member gives to leave.
pub const CHALLENGE_WINDOW_SECONDS: i64 = 30 * 24 * 60 * 60;
pub const ANSWER_WINDOW_SECONDS: i64 = 7 * 24 * 60 * 60;
/// The largest reply a member can put on chain in answer. An account created
/// by the program is at most 10,240 bytes.
pub const MAX_REPLY_BYTES: u32 = 10_000;
pub const CHALLENGE_OPEN: u8 = 0;
pub const CHALLENGE_ANSWERED: u8 = 1;
pub const CHALLENGE_UPHELD: u8 = 2;
/// A reading whose challenge was upheld. It no longer counts.
pub const READING_FALSE: u8 = 3;

/// Quotes (SPEC.md section 6): what the member who read a service would
/// charge to insure a purchase from it, in basis points of its price.
pub const QUOTE_SEED: &[u8] = b"quote";
pub const MAX_QUOTE_BPS: u16 = 10_000;

/// NOTE, temporary: who may approve a claim. Deciding claims belongs to
/// members drawn at random, which is part 7 of the roadmap. Until then it is
/// the key that can already upgrade this program, so nothing new is trusted.
pub const JUDGE: Pubkey = pubkey!("CCsLKV9yCucqYmdb1KarjsTD5pA6q2fzucBa9ozCTSFu");

#[program]
pub mod sasona {
    use super::*;

    /// Create the coin and the pool, and make the first deposit.
    ///
    /// There is no other way to start. The opener pays the entry fee and
    /// locks a guarantee like any later depositor, and coins are only minted
    /// against what was deposited. Anyone may be the opener: the dollar and
    /// the price are fixed in the program, so going first gains nothing.
    pub fn open(ctx: Context<Open>, amount: u64) -> Result<()> {
        require!(amount > 0, SasonaError::NothingDeposited);
        let usd_mint = USD_MINT;
        let coins_per_usd = OPENING_COINS_PER_USD;

        let s = Slices::of(amount)?;

        let pool = &mut ctx.accounts.pool;
        pool.bump = ctx.bumps.pool;
        pool.usd_mint = usd_mint;
        pool.coin_mint = ctx.accounts.coin_mint.key();

        // Every amount below is a whole multiple of the opening price, so
        // nothing rounds.
        let into_pool = times(s.rest, coins_per_usd)?;
        let free_coins = times(s.free, coins_per_usd)?;
        let guarantee_coins = times(s.guarantee, coins_per_usd)?;

        let bump = [ctx.accounts.pool.bump];
        let seeds: &[&[u8]] = &[POOL_SEED, &bump];
        let signer: &[&[&[u8]]] = &[seeds];

        let a = &ctx.accounts;
        token::transfer(a.cpi_transfer(&a.depositor_usd, &a.pool_usd, a.depositor.to_account_info()), s.rest)?;
        token::transfer(a.cpi_transfer(&a.depositor_usd, &a.fees, a.depositor.to_account_info()), s.fee)?;
        token::mint_to(a.cpi_mint(&a.pool_coin).with_signer(signer), into_pool)?;
        token::mint_to(a.cpi_mint(&a.depositor_coin).with_signer(signer), free_coins)?;
        token::mint_to(a.cpi_mint(&a.cover_vault).with_signer(signer), guarantee_coins)?;

        let pool = &mut ctx.accounts.pool;
        pool.usd_reserve = s.rest;
        pool.coin_reserve = into_pool;
        pool.outside = add(free_coins, guarantee_coins)?;
        pool.fees_held = s.fee;

        let cover = &mut ctx.accounts.cover;
        cover.bump = ctx.bumps.cover;
        let shares = cover.take_in(guarantee_coins)?;

        let g = &mut ctx.accounts.guarantee;
        g.owner = ctx.accounts.depositor.key();
        g.shares = shares;
        g.bump = ctx.bumps.guarantee;

        ctx.accounts.coin_mint.reload()?;
        ctx.accounts.pool_usd.reload()?;
        ctx.accounts.fees.reload()?;
        ctx.accounts.cover_vault.reload()?;
        let a = &ctx.accounts;
        a.pool.check(a.coin_mint.supply, a.pool_usd.amount, a.fees.amount)?;
        a.cover.check(a.cover_vault.amount)?;

        emit!(Opened {
            usd_mint,
            coin_mint: a.pool.coin_mint,
            depositor: a.depositor.key(),
            amount,
            coins_per_usd,
        });
        Ok(())
    }

    /// Deposit into an open pool.
    ///
    /// The same four slices as the opening, priced at the pool's own ratio of
    /// coins to dollars. The pool's side is minted to match what it receives,
    /// so a deposit does not move the price. Rounding always goes down: the
    /// depositor can receive a fraction of a coin unit less, never more, and
    /// the price can only drift up, by less than one unit.
    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        require!(amount > 0, SasonaError::NothingDeposited);
        require!(ctx.accounts.legacy_vault.data_is_empty(), SasonaError::JoinCoverFirst);
        let s = Slices::of(amount)?;

        let (usd_before, coin_before) = (ctx.accounts.pool.usd_reserve, ctx.accounts.pool.coin_reserve);
        let into_pool = at_price(s.rest, coin_before, usd_before)?;
        let free_coins = at_price(s.free, coin_before, usd_before)?;
        let guarantee_coins = at_price(s.guarantee, coin_before, usd_before)?;
        require!(s.fee > 0 && free_coins > 0 && guarantee_coins > 0, SasonaError::TooSmall);

        let bump = [ctx.accounts.pool.bump];
        let seeds: &[&[u8]] = &[POOL_SEED, &bump];
        let signer: &[&[&[u8]]] = &[seeds];

        let a = &ctx.accounts;
        token::transfer(a.cpi_transfer(&a.depositor_usd, &a.pool_usd, a.depositor.to_account_info()), s.rest)?;
        token::transfer(a.cpi_transfer(&a.depositor_usd, &a.fees, a.depositor.to_account_info()), s.fee)?;
        token::mint_to(a.cpi_mint(&a.pool_coin).with_signer(signer), into_pool)?;
        token::mint_to(a.cpi_mint(&a.depositor_coin).with_signer(signer), free_coins)?;
        token::mint_to(a.cpi_mint(&a.cover_vault).with_signer(signer), guarantee_coins)?;

        let pool = &mut ctx.accounts.pool;
        pool.usd_reserve = add(usd_before, s.rest)?;
        pool.coin_reserve = add(coin_before, into_pool)?;
        pool.outside = add(pool.outside, add(free_coins, guarantee_coins)?)?;
        pool.fees_held = add(pool.fees_held, s.fee)?;
        pool.price_held(usd_before, coin_before)?;

        let shares = ctx.accounts.cover.take_in(guarantee_coins)?;

        // A first deposit creates the guarantee; a later one adds to it.
        let g = &mut ctx.accounts.guarantee;
        if g.owner == Pubkey::default() {
            g.owner = ctx.accounts.depositor.key();
            g.bump = ctx.bumps.guarantee;
        }
        g.shares = add(g.shares, shares)?;

        ctx.accounts.coin_mint.reload()?;
        ctx.accounts.pool_usd.reload()?;
        ctx.accounts.fees.reload()?;
        ctx.accounts.cover_vault.reload()?;
        let a = &ctx.accounts;
        a.pool.check(a.coin_mint.supply, a.pool_usd.amount, a.fees.amount)?;
        a.cover.check(a.cover_vault.amount)?;

        emit!(Deposited {
            depositor: a.depositor.key(),
            amount,
            into_pool,
            free_coins,
            guarantee_coins,
        });
        Ok(())
    }

    /// Pay the markup on a purchase, in dollars.
    ///
    /// `markup` is the fifteen points, not the seller's price. The reserve
    /// comes off the top and is added to the pool as depth first. The rest
    /// then buys coin from the deeper pool, which moves the price up; the
    /// burn's share of that coin is destroyed and the remainder goes to the
    /// participants.
    ///
    /// Not yet tied to a real purchase: any caller can pay any markup. While
    /// every coin it buys goes to the network vault that is harmless. It must
    /// be tied to purchases before participant seats are paid, or paying a
    /// "fee" becomes a way for a participant to buy coin.
    pub fn pay_fee(ctx: Context<PayFee>, markup: u64) -> Result<()> {
        require!(markup > 0, SasonaError::NothingDeposited);
        let f = Fee::of(markup)?;

        let a = &ctx.accounts;
        token::transfer(
            CpiContext::new(
                a.token_program.key(),
                Transfer {
                    from: a.payer_usd.to_account_info(),
                    to: a.pool_usd.to_account_info(),
                    authority: a.payer.to_account_info(),
                },
            ),
            markup,
        )?;

        let pool = &mut ctx.accounts.pool;
        pool.usd_reserve = add(pool.usd_reserve, f.reserve)?;
        let (burned, shared) = buy_and_share(
            &mut ctx.accounts.pool,
            &ctx.accounts.token_program,
            &ctx.accounts.coin_mint,
            &ctx.accounts.pool_coin,
            &ctx.accounts.network,
            f.buy(),
            f.burn,
        )?;

        ctx.accounts.coin_mint.reload()?;
        ctx.accounts.pool_usd.reload()?;
        let a = &ctx.accounts;
        a.pool.check(a.coin_mint.supply, a.pool_usd.amount, a.fees.amount)?;

        emit!(FeePaid { payer: a.payer.key(), markup, reserve: f.reserve, burned, shared });
        Ok(())
    }

    /// Turn the entry fees that deposits have left waiting into coin.
    ///
    /// The same as a fee, without the reserve, which belongs to the markup on
    /// purchases only. Anyone may call it; it moves only the pool's own money.
    pub fn settle_entry_fees(ctx: Context<SettleEntryFees>) -> Result<()> {
        let amount = ctx.accounts.pool.fees_held;
        require!(amount > 0, SasonaError::NothingDeposited);
        let burn = burn_of(amount)?;

        let bump = [ctx.accounts.pool.bump];
        let seeds: &[&[u8]] = &[POOL_SEED, &bump];
        let signer: &[&[&[u8]]] = &[seeds];
        let a = &ctx.accounts;
        token::transfer(
            CpiContext::new(
                a.token_program.key(),
                Transfer {
                    from: a.fees.to_account_info(),
                    to: a.pool_usd.to_account_info(),
                    authority: a.pool.to_account_info(),
                },
            )
            .with_signer(signer),
            amount,
        )?;
        ctx.accounts.pool.fees_held = 0;

        let (burned, shared) = buy_and_share(
            &mut ctx.accounts.pool,
            &ctx.accounts.token_program,
            &ctx.accounts.coin_mint,
            &ctx.accounts.pool_coin,
            &ctx.accounts.network,
            amount,
            burn,
        )?;

        ctx.accounts.coin_mint.reload()?;
        ctx.accounts.pool_usd.reload()?;
        ctx.accounts.fees.reload()?;
        let a = &ctx.accounts;
        a.pool.check(a.coin_mint.supply, a.pool_usd.amount, a.fees.amount)?;

        emit!(EntryFeesSettled { amount, burned, shared });
        Ok(())
    }

    /// Add dollars to the pool and nothing else.
    ///
    /// Nothing is minted against them, so the same coins are backed by more
    /// money and the price rises. It is the one way to make the pool absorb
    /// a larger sale without handing anybody a claim on it. Anyone may call
    /// it, because a caller can only give.
    pub fn add_depth(ctx: Context<AddDepth>, amount: u64) -> Result<()> {
        require!(amount > 0, SasonaError::NothingDeposited);
        let a = &ctx.accounts;
        token::transfer(
            CpiContext::new(
                a.token_program.key(),
                Transfer {
                    from: a.giver_usd.to_account_info(),
                    to: a.pool_usd.to_account_info(),
                    authority: a.giver.to_account_info(),
                },
            ),
            amount,
        )?;
        let pool = &mut ctx.accounts.pool;
        pool.usd_reserve = add(pool.usd_reserve, amount)?;

        ctx.accounts.pool_usd.reload()?;
        let a = &ctx.accounts;
        a.pool.check(a.coin_mint.supply, a.pool_usd.amount, a.fees.amount)?;

        emit!(DepthAdded { giver: a.giver.key(), amount });
        Ok(())
    }

    /// Commit a round: the list of services by its fingerprint and size, how
    /// many will be drawn, and the hash of a seed only the opener knows.
    ///
    /// The round's address comes from the list's fingerprint, so a list can
    /// be drawn once. Otherwise an opener could open several rounds for the
    /// same list, reveal them all and keep whichever result they liked.
    pub fn open_round(
        ctx: Context<OpenRound>,
        pool_fingerprint: [u8; 32],
        pool_size: u32,
        count: u16,
        seed_hash: [u8; 32],
    ) -> Result<()> {
        require!(
            pool_size > 0 && pool_size <= MAX_ROUND_CANDIDATES && count > 0 && count as u32 <= pool_size,
            SasonaError::BadRound
        );
        anchor_lang::system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.key(),
                anchor_lang::system_program::Transfer {
                    from: ctx.accounts.opener.to_account_info(),
                    to: ctx.accounts.round.to_account_info(),
                },
            ),
            ROUND_BOND_LAMPORTS,
        )?;
        let r = &mut ctx.accounts.round;
        r.opener = ctx.accounts.opener.key();
        r.pool_fingerprint = pool_fingerprint;
        r.pool_size = pool_size;
        r.count = count;
        r.seed_hash = seed_hash;
        r.commit_slot = Clock::get()?.slot;
        r.state = ROUND_COMMITTED;
        r.bump = ctx.bumps.round;
        // SPEC.md 4.2: the roster is the memberships taken before now.
        r.members = ctx.accounts.members.seated;
        ctx.accounts.members.bump = ctx.bumps.members;
        emit!(RoundOpened { round: r.key(), opener: r.opener, pool_fingerprint, pool_size, count, commit_slot: r.commit_slot });
        Ok(())
    }

    /// Reveal a round's seed and fix its final seed.
    ///
    /// The entropy is the hash of the earliest slot at or after the target
    /// slot, read from the SlotHashes sysvar. It can be read only while
    /// SlotHashes still reaches back that far, a few minutes. Anyone holding
    /// the seed may reveal it.
    pub fn reveal_round(ctx: Context<RevealRound>, seed: [u8; 32]) -> Result<()> {
        let r = &mut ctx.accounts.round;
        require!(r.state == ROUND_COMMITTED, SasonaError::RoundNotOpen);
        require!(solana_sha256_hasher::hashv(&[&seed]).to_bytes() == r.seed_hash, SasonaError::WrongSeed);

        let target = r.commit_slot + ENTROPY_DELAY_SLOTS;
        let data = ctx.accounts.slot_hashes.try_borrow_data()?;
        let (entropy_slot, entropy) = match entropy_for(&data, target)? {
            Entropy::Found(slot, hash) => (slot, hash),
            Entropy::TooEarly => return err!(SasonaError::TooEarly),
            Entropy::Gone => return err!(SasonaError::TooLate),
        };

        r.seed = seed;
        r.entropy_slot = entropy_slot;
        r.entropy = entropy;
        r.final_seed = solana_sha256_hasher::hashv(&[DRAW_DOMAIN, &seed, &entropy]).to_bytes();
        r.state = ROUND_DRAWN;

        // The bond goes back to the opener. The round keeps its rent and stays.
        let round_info = ctx.accounts.round.to_account_info();
        **round_info.try_borrow_mut_lamports()? -= ROUND_BOND_LAMPORTS;
        **ctx.accounts.opener.to_account_info().try_borrow_mut_lamports()? += ROUND_BOND_LAMPORTS;
        let r = &ctx.accounts.round;
        emit!(RoundDrawn { round: r.key(), entropy_slot, final_seed: r.final_seed });
        Ok(())
    }

    /// Mark a round whose seed was never revealed while it still could be.
    /// It can never be drawn after this, and the record stays.
    pub fn mark_withheld(ctx: Context<MarkWithheld>) -> Result<()> {
        let r = &mut ctx.accounts.round;
        require!(r.state == ROUND_COMMITTED, SasonaError::RoundNotOpen);
        let target = r.commit_slot + ENTROPY_DELAY_SLOTS;
        let data = ctx.accounts.slot_hashes.try_borrow_data()?;
        require!(matches!(entropy_for(&data, target)?, Entropy::Gone), SasonaError::NotWithheldYet);
        r.state = ROUND_WITHHELD;
        emit!(RoundWithheld { round: r.key() });
        Ok(())
    }

    /// Commit to the question for one service of a drawn round, before
    /// calling it.
    ///
    /// Only the member drawn for the service may (SPEC.md 4.3), within about
    /// an hour of the draw. The seats drawn before theirs and passed over come
    /// in the remaining accounts, in order, so the program can check each.
    /// Whether the service is one of the round's picks is checked off chain,
    /// against the published list.
    pub fn commit_reading<'info>(
        ctx: Context<'info, CommitReading<'info>>,
        endpoint_hash: [u8; 32],
        endpoint: String,
        question_hash: [u8; 32],
    ) -> Result<()> {
        let a = &ctx.accounts;
        check_drawn(&a.round, &a.members, &endpoint_hash, &a.member, &a.seat, a.reader.key(), ctx.remaining_accounts, None)?;
        let a = &mut *ctx.accounts;
        start_reading(&a.round, &mut a.reading, a.reader.key(), ctx.bumps.reading, endpoint_hash, endpoint, question_hash)?;
        a.reading.member = a.member.number;
        Ok(())
    }

    /// Reveal the nonce, the reply's hash and the verdict.
    ///
    /// The program builds the question from the nonce itself (SPEC.md 2.3)
    /// and refuses unless it is the one committed, so only the fair question
    /// for this nonce can be revealed.
    ///
    /// A nonce belongs to the reading that committed to it earliest. The
    /// service sees the nonce when it is called, after the honest reading was
    /// committed, so it can copy the nonce into a reading of its own but never
    /// one committed earlier. If the copy is revealed first, the honest reveal
    /// takes the nonce back, and the copy no longer counts. A reading whose
    /// nonce is held by an earlier one cannot be revealed.
    ///
    /// The verdict is checked by anyone holding the reply, against SPEC.md 2.4.
    pub fn reveal_reading(ctx: Context<RevealReading>, nonce: [u8; 16], reply_hash: [u8; 32], verdict: u8) -> Result<()> {
        let r = &mut ctx.accounts.reading;
        require!(r.state == READING_COMMITTED, SasonaError::ReadingNotOpen);
        let now = Clock::get()?.slot;
        require!(now > r.commit_slot, SasonaError::TooEarly);
        require!(now <= r.commit_slot + REVEAL_WINDOW_SLOTS, SasonaError::TooLate);
        let question = canonical_question(&nonce);
        require!(solana_sha256_hasher::hashv(&[&question]).to_bytes() == r.question_hash, SasonaError::WrongQuestion);
        require!((1..=3).contains(&verdict), SasonaError::BadVerdict);

        let this = r.key();
        let committed = r.commit_slot;
        let used = &mut ctx.accounts.used_nonce;
        if used.reading != Pubkey::default() {
            // Held already: only a reading committed strictly earlier takes it.
            require!(committed < used.commit_slot, SasonaError::NonceTaken);
        }
        used.reading = this;
        used.commit_slot = committed;

        let r = &mut ctx.accounts.reading;
        r.reply_hash = reply_hash;
        r.verdict = verdict;
        r.reveal_slot = now;
        r.reveal_time = Clock::get()?.unix_timestamp;
        r.state = READING_REVEALED;
        emit!(ReadingRevealed { reading: r.key(), reply_hash, verdict, reveal_slot: now });
        Ok(())
    }

    /// Commit a second reading: an ordinary reading in a re-read round that
    /// names the first reading it re-tests (sasona-protocol SPEC.md 3.2).
    /// It must be the same service, read by someone else, in another round,
    /// after the first was revealed.
    pub fn commit_second_reading<'info>(
        ctx: Context<'info, CommitSecondReading<'info>>,
        endpoint_hash: [u8; 32],
        endpoint: String,
        question_hash: [u8; 32],
    ) -> Result<()> {
        let first = &ctx.accounts.first;
        require!(first.state == READING_REVEALED, SasonaError::FirstNotRevealed);
        require!(first.endpoint == endpoint, SasonaError::NotTheSameService);
        // Already impossible: in the first's round, the reading address for
        // this service is the first reading itself. Kept so the rule reads
        // here as it does in the specification.
        require!(first.round != ctx.accounts.round.key(), SasonaError::SameRound);
        require!(first.reader != ctx.accounts.reader.key(), SasonaError::SameReader);
        // The first is the latest reading of the service revealed before the
        // re-read round was committed (SPEC.md 3.2). That it came before is
        // checked here; that it is the latest is checked off chain (3.5).
        require!(first.reveal_slot < ctx.accounts.round.commit_slot, SasonaError::TooEarly);
        let first_key = first.key();
        let first_reader = first.reader;
        let a = &ctx.accounts;
        check_drawn(&a.round, &a.members, &endpoint_hash, &a.member, &a.seat, a.reader.key(), ctx.remaining_accounts, Some(first_reader))?;

        let a = &mut *ctx.accounts;
        start_reading(&a.round, &mut a.reading, a.reader.key(), ctx.bumps.reading, endpoint_hash, endpoint, question_hash)?;
        a.reading.member = a.member.number;
        let p = &mut a.pair;
        p.first = first_key;
        p.second = a.reading.key();
        p.outcome = 0;
        p.bump = ctx.bumps.pair;
        Ok(())
    }

    /// Record what a pair settled, once its second reading is revealed.
    /// Anyone may do it; it only reads the two verdicts.
    pub fn settle_pair(ctx: Context<SettlePair>) -> Result<()> {
        let second = &ctx.accounts.second;
        require!(second.state == READING_REVEALED, SasonaError::ReadingNotOpen);
        // A first reading upheld false since no longer counts (SPEC.md 5.3).
        require!(ctx.accounts.first.state == READING_REVEALED, SasonaError::FirstNotRevealed);
        let p = &mut ctx.accounts.pair;
        require!(p.outcome == 0, SasonaError::AlreadySettled);
        p.outcome = pair_outcome(ctx.accounts.first.verdict, second.verdict);
        emit!(PairSettled { pair: p.key(), first: p.first, second: p.second, outcome: p.outcome });
        Ok(())
    }

    /// Mark a reading whose question was not revealed in time.
    pub fn mark_lapsed(ctx: Context<MarkLapsed>) -> Result<()> {
        let r = &mut ctx.accounts.reading;
        require!(r.state == READING_COMMITTED, SasonaError::ReadingNotOpen);
        require!(Clock::get()?.slot > r.commit_slot + REVEAL_WINDOW_SLOTS, SasonaError::NotLapsedYet);
        r.state = READING_LAPSED;
        emit!(ReadingLapsed { reading: r.key() });
        Ok(())
    }

    /// Take a membership: lock one stake of coin (SPEC.md 4.1). The
    /// membership gets the next number and sits in the seat after the last.
    pub fn join_members(ctx: Context<JoinMembers>) -> Result<()> {
        let a = &ctx.accounts;
        token::transfer(
            CpiContext::new(
                a.token_program.key(),
                Transfer {
                    from: a.owner_coin.to_account_info(),
                    to: a.stakes.to_account_info(),
                    authority: a.owner.to_account_info(),
                },
            ),
            MEMBER_STAKE,
        )?;
        let members = &mut ctx.accounts.members;
        members.bump = ctx.bumps.members;
        members.count = members.count.checked_add(1).ok_or(SasonaError::Overflow)?;
        members.seated = members.seated.checked_add(1).ok_or(SasonaError::Overflow)?;
        let (number, seat) = (members.count, members.seated);
        let owner = ctx.accounts.owner.key();
        let m = &mut ctx.accounts.member;
        m.owner = owner;
        m.number = number;
        m.seat = seat;
        m.state = MEMBER_ACTIVE;
        m.stake = MEMBER_STAKE;
        m.leave_at = 0;
        m.open_challenges = 0;
        m.bump = ctx.bumps.member;
        let s = &mut ctx.accounts.seat;
        s.member = number;
        s.owner = owner;
        s.since = Clock::get()?.slot;
        s.bump = ctx.bumps.seat;
        emit!(MemberJoined { owner, number, seat, stake: MEMBER_STAKE });
        Ok(())
    }

    /// Ask to leave. The membership leaves its seat now; its stake comes back
    /// after the notice (SPEC.md 4.4). The remaining accounts are its seat,
    /// then, unless it sat last, the last seat and the membership in it.
    pub fn ask_to_leave<'info>(ctx: Context<'info, AskToLeave<'info>>) -> Result<()> {
        require!(ctx.accounts.member.state == MEMBER_ACTIVE, SasonaError::NotActive);
        unseat(&mut ctx.accounts.members, &mut ctx.accounts.member, ctx.remaining_accounts)?;
        let m = &mut ctx.accounts.member;
        m.state = MEMBER_LEAVING;
        m.leave_at = Clock::get()?.unix_timestamp.checked_add(NOTICE_SECONDS).ok_or(SasonaError::Overflow)?;
        emit!(MemberLeaving { number: m.number, leave_at: m.leave_at });
        Ok(())
    }

    /// Take the stake back once the notice has run out and no challenge to
    /// one of the membership's readings is open.
    pub fn leave(ctx: Context<Leave>) -> Result<()> {
        let m = &ctx.accounts.member;
        require!(m.state == MEMBER_LEAVING, SasonaError::NotLeaving);
        require!(Clock::get()?.unix_timestamp >= m.leave_at, SasonaError::NoticeNotOver);
        require!(m.open_challenges == 0, SasonaError::ChallengeOpen);
        let stake = m.stake;

        let bump = [ctx.accounts.pool.bump];
        let seeds: &[&[u8]] = &[POOL_SEED, &bump];
        let signer: &[&[&[u8]]] = &[seeds];
        let a = &ctx.accounts;
        token::transfer(
            CpiContext::new(
                a.token_program.key(),
                Transfer { from: a.stakes.to_account_info(), to: a.owner_coin.to_account_info(), authority: a.pool.to_account_info() },
            )
            .with_signer(signer),
            stake,
        )?;
        let m = &mut ctx.accounts.member;
        m.state = MEMBER_LEFT;
        m.stake = 0;
        emit!(MemberLeft { number: m.number, stake });
        Ok(())
    }

    /// Challenge a reading taken by a member, within 30 days of its reveal:
    /// they have 7 days to put the nonce and the reply on chain (SPEC.md 5.1).
    /// A reading can be challenged once. The bond goes to the member if they
    /// answer.
    pub fn challenge(ctx: Context<ChallengeReading>) -> Result<()> {
        let r = &ctx.accounts.reading;
        require!(r.state == READING_REVEALED, SasonaError::ReadingNotOpen);
        require!(r.member > 0, SasonaError::NoMember);
        let now = Clock::get()?.unix_timestamp;
        require!(now <= r.reveal_time.checked_add(CHALLENGE_WINDOW_SECONDS).ok_or(SasonaError::Overflow)?, SasonaError::WindowClosed);
        anchor_lang::system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.key(),
                anchor_lang::system_program::Transfer {
                    from: ctx.accounts.challenger.to_account_info(),
                    to: ctx.accounts.challenge.to_account_info(),
                },
            ),
            CHALLENGE_BOND_LAMPORTS,
        )?;
        let deadline = now.checked_add(ANSWER_WINDOW_SECONDS).ok_or(SasonaError::Overflow)?;
        let c = &mut ctx.accounts.challenge;
        c.reading = r.key();
        c.challenger = ctx.accounts.challenger.key();
        c.deadline = deadline;
        c.state = CHALLENGE_OPEN;
        c.bump = ctx.bumps.challenge;
        let m = &mut ctx.accounts.member;
        m.open_challenges = m.open_challenges.checked_add(1).ok_or(SasonaError::Overflow)?;
        emit!(Challenged { reading: c.reading, challenger: c.challenger, deadline });
        Ok(())
    }

    /// Make room on chain for a reading's reply, `len` bytes, all zero until
    /// written. Only the reader can.
    pub fn open_evidence(ctx: Context<OpenEvidence>, len: u32) -> Result<()> {
        require!(len <= MAX_REPLY_BYTES, SasonaError::ReplyTooLong);
        let e = &mut ctx.accounts.evidence;
        e.reading = ctx.accounts.reading.key();
        e.sealed = false;
        e.reply = vec![0u8; len as usize];
        Ok(())
    }

    /// Write part of the reply, from `offset`, until an answer seals it.
    pub fn write_evidence(ctx: Context<WriteEvidence>, offset: u32, bytes: Vec<u8>) -> Result<()> {
        let e = &mut ctx.accounts.evidence;
        require!(!e.sealed, SasonaError::EvidenceSealed);
        let start = offset as usize;
        let end = start.checked_add(bytes.len()).ok_or(SasonaError::Overflow)?;
        require!(end <= e.reply.len(), SasonaError::ReplyTooLong);
        e.reply[start..end].copy_from_slice(&bytes);
        Ok(())
    }

    /// Answer a challenge with the nonce, against the reply already written
    /// (SPEC.md 5.2). Anyone may send it; it holds or it does not. Once it
    /// holds, the reply on chain can no longer be changed.
    pub fn answer_challenge(ctx: Context<AnswerChallenge>, nonce: [u8; 16]) -> Result<()> {
        let c = &ctx.accounts.challenge;
        require!(c.state == CHALLENGE_OPEN, SasonaError::ChallengeClosed);
        require!(Clock::get()?.unix_timestamp <= c.deadline, SasonaError::WindowClosed);
        let r = &ctx.accounts.reading;
        require_keys_eq!(ctx.accounts.used_nonce.reading, r.key(), SasonaError::NonceTaken);
        let reply = &ctx.accounts.evidence.reply;
        require!(solana_sha256_hasher::hashv(&[reply]).to_bytes() == r.reply_hash, SasonaError::NotTheReply);
        require!(verdict_of(reply, &nonce) == r.verdict, SasonaError::VerdictDoesNotFollow);

        let challenge_info = ctx.accounts.challenge.to_account_info();
        **challenge_info.try_borrow_mut_lamports()? -= CHALLENGE_BOND_LAMPORTS;
        **ctx.accounts.reader.to_account_info().try_borrow_mut_lamports()? += CHALLENGE_BOND_LAMPORTS;
        ctx.accounts.challenge.state = CHALLENGE_ANSWERED;
        ctx.accounts.evidence.sealed = true;
        let m = &mut ctx.accounts.member;
        m.open_challenges -= 1;
        emit!(ChallengeAnswered { reading: ctx.accounts.reading.key() });
        Ok(())
    }

    /// Uphold a challenge nobody answered in time (SPEC.md 5.3). The reading
    /// stops counting. If the membership still has its stake, it loses all of
    /// it, a tenth to the challenger and the rest held, and its seat if it
    /// has one: then the remaining accounts are as for ask_to_leave. Anyone
    /// may send it.
    pub fn uphold_challenge<'info>(ctx: Context<'info, UpholdChallenge<'info>>) -> Result<()> {
        let c = &ctx.accounts.challenge;
        require!(c.state == CHALLENGE_OPEN, SasonaError::ChallengeClosed);
        require!(Clock::get()?.unix_timestamp > c.deadline, SasonaError::NotLapsedYet);

        let state = ctx.accounts.member.state;
        let stake = if state == MEMBER_ACTIVE || state == MEMBER_LEAVING { ctx.accounts.member.stake } else { 0 };
        let reward = stake / 10;
        if stake > 0 {
            let bump = [ctx.accounts.pool.bump];
            let seeds: &[&[u8]] = &[POOL_SEED, &bump];
            let signer: &[&[&[u8]]] = &[seeds];
            let a = &ctx.accounts;
            for (to, amount) in [(a.challenger_coin.to_account_info(), reward), (a.held.to_account_info(), stake - reward)] {
                token::transfer(
                    CpiContext::new(
                        a.token_program.key(),
                        Transfer { from: a.stakes.to_account_info(), to, authority: a.pool.to_account_info() },
                    )
                    .with_signer(signer),
                    amount,
                )?;
            }
        }
        if state == MEMBER_ACTIVE {
            unseat(&mut ctx.accounts.members, &mut ctx.accounts.member, ctx.remaining_accounts)?;
        }

        let challenge_info = ctx.accounts.challenge.to_account_info();
        **challenge_info.try_borrow_mut_lamports()? -= CHALLENGE_BOND_LAMPORTS;
        **ctx.accounts.challenger.to_account_info().try_borrow_mut_lamports()? += CHALLENGE_BOND_LAMPORTS;
        ctx.accounts.challenge.state = CHALLENGE_UPHELD;
        ctx.accounts.reading.state = READING_FALSE;
        let m = &mut ctx.accounts.member;
        m.open_challenges -= 1;
        if stake > 0 {
            m.state = MEMBER_SLASHED;
            m.stake = 0;
        }
        emit!(ChallengeUpheld { reading: ctx.accounts.reading.key(), number: m.number, stake_taken: stake });
        Ok(())
    }

    /// Set, change or withdraw (`rate` 0) the quote on a reading (SPEC.md 6.1).
    /// Only its reader can, and a rate only on a reading that counts, says
    /// delivered, is still current, and whose membership is active.
    pub fn set_quote(ctx: Context<SetQuote>, rate: u16) -> Result<()> {
        let r = &ctx.accounts.reading;
        let clock = Clock::get()?;
        if rate > 0 {
            require!(rate <= MAX_QUOTE_BPS, SasonaError::BadRate);
            require!(r.state == READING_REVEALED, SasonaError::ReadingNotOpen);
            require!(r.verdict == 1, SasonaError::NotDelivered);
            require!(r.member > 0, SasonaError::NoMember);
            require!(ctx.accounts.member.state == MEMBER_ACTIVE, SasonaError::NotActive);
            let ends = r.reveal_time.checked_add(CHALLENGE_WINDOW_SECONDS).ok_or(SasonaError::Overflow)?;
            require!(clock.unix_timestamp <= ends, SasonaError::WindowClosed);
        }
        let q = &mut ctx.accounts.quote;
        q.reading = r.key();
        q.member = r.member;
        q.rate = rate;
        q.set_slot = clock.slot;
        q.set_time = clock.unix_timestamp;
        q.bump = ctx.bumps.quote;
        emit!(QuoteSet { reading: q.reading, member: q.member, rate, set_time: q.set_time });
        Ok(())
    }

    /// Move a guarantee made before the cover existed into the cover.
    ///
    /// A one-off for the guarantees already on devnet. The coins move from
    /// the depositor's old vault, which is then closed with its rent going
    /// back to them, and the record is converted from coins to shares.
    /// Anyone may call it, because it changes nothing anybody owns.
    pub fn join_cover(ctx: Context<JoinCover>) -> Result<()> {
        // Everything in the old vault moves, including anything someone else
        // sent to it: requiring an exact match would let one stray coin unit
        // block the move for ever.
        let coins = ctx.accounts.legacy_vault.amount;
        require!(coins > 0, SasonaError::NothingDeposited);

        let bump = [ctx.accounts.pool.bump];
        let seeds: &[&[u8]] = &[POOL_SEED, &bump];
        let signer: &[&[&[u8]]] = &[seeds];
        let a = &ctx.accounts;
        token::transfer(
            CpiContext::new(
                a.token_program.key(),
                Transfer {
                    from: a.legacy_vault.to_account_info(),
                    to: a.cover_vault.to_account_info(),
                    authority: a.pool.to_account_info(),
                },
            )
            .with_signer(signer),
            coins,
        )?;
        token::close_account(
            CpiContext::new(
                a.token_program.key(),
                CloseAccount {
                    account: a.legacy_vault.to_account_info(),
                    destination: a.owner.to_account_info(),
                    authority: a.pool.to_account_info(),
                },
            )
            .with_signer(signer),
        )?;

        let cover = &mut ctx.accounts.cover;
        cover.bump = ctx.bumps.cover;
        let shares = cover.take_in(coins)?;
        ctx.accounts.guarantee.shares = shares;

        ctx.accounts.cover_vault.reload()?;
        let a = &ctx.accounts;
        a.cover.check(a.cover_vault.amount)?;
        emit!(JoinedCover { owner: a.owner.key(), coins, shares });
        Ok(())
    }

    /// Pay a buyer back out of the cover.
    ///
    /// The dollars leave the pool, and two equal amounts of coin are burned:
    /// the pool's side, so the price does not fall, and the cover's, because
    /// the guarantees are what paid. It undoes, for this many dollars, what a
    /// deposit's guarantee slice did. `reference` names the purchase.
    pub fn claim(ctx: Context<Claim>, dollars: u64, reference: [u8; 32]) -> Result<()> {
        require_keys_eq!(ctx.accounts.judge.key(), JUDGE, SasonaError::NotTheJudge);
        require!(dollars > 0, SasonaError::NothingDeposited);

        let (usd, coins) = (ctx.accounts.pool.usd_reserve, ctx.accounts.pool.coin_reserve);
        require!(dollars < usd, SasonaError::MoreThanIsThere);
        // Rounded up, so the pool gives up at least the coins behind the
        // dollars and the price can only rise.
        let burn = u64::try_from((dollars as u128 * coins as u128).div_ceil(usd as u128))
            .map_err(|_| SasonaError::Overflow)?;
        // The cover never empties, and never gets so thin that new shares
        // would overflow; see MAX_SHARES_PER_COIN.
        let cover = &ctx.accounts.cover;
        require!(burn < coins && burn < cover.coins, SasonaError::MoreThanIsThere);
        require!(
            (cover.coins - burn) as u128 * MAX_SHARES_PER_COIN as u128 >= cover.shares as u128,
            SasonaError::CoverTooThin
        );

        let bump = [ctx.accounts.pool.bump];
        let seeds: &[&[u8]] = &[POOL_SEED, &bump];
        let signer: &[&[&[u8]]] = &[seeds];
        let a = &ctx.accounts;
        token::transfer(
            CpiContext::new(
                a.token_program.key(),
                Transfer {
                    from: a.pool_usd.to_account_info(),
                    to: a.claimant_usd.to_account_info(),
                    authority: a.pool.to_account_info(),
                },
            )
            .with_signer(signer),
            dollars,
        )?;
        for from in [a.pool_coin.to_account_info(), a.cover_vault.to_account_info()] {
            token::burn(
                CpiContext::new(
                    a.token_program.key(),
                    Burn { mint: a.coin_mint.to_account_info(), from, authority: a.pool.to_account_info() },
                )
                .with_signer(signer),
                burn,
            )?;
        }

        let pool = &mut ctx.accounts.pool;
        pool.usd_reserve = usd - dollars;
        pool.coin_reserve = coins - burn;
        pool.outside = pool.outside.checked_sub(burn).ok_or(SasonaError::Overflow)?;
        require!(
            pool.usd_reserve as u128 * coins as u128 >= usd as u128 * pool.coin_reserve as u128,
            SasonaError::PriceMoved
        );
        let cover = &mut ctx.accounts.cover;
        cover.coins -= burn;

        ctx.accounts.coin_mint.reload()?;
        ctx.accounts.pool_usd.reload()?;
        ctx.accounts.cover_vault.reload()?;
        let a = &ctx.accounts;
        a.pool.check(a.coin_mint.supply, a.pool_usd.amount, a.fees.amount)?;
        a.cover.check(a.cover_vault.amount)?;

        emit!(Claimed { claimant: a.claimant_usd.owner, dollars, coins_burned: burn.saturating_mul(2), reference });
        Ok(())
    }

    /// Ask for some of your guarantee back.
    ///
    /// The shares leave your guarantee but stay in the cover, paying claims
    /// like any other, until the notice runs out. Asking again adds to what
    /// is waiting and starts the notice again.
    pub fn request_release(ctx: Context<RequestRelease>, shares: u64) -> Result<()> {
        require!(shares > 0, SasonaError::NothingDeposited);
        require!(ctx.accounts.legacy_vault.data_is_empty(), SasonaError::JoinCoverFirst);
        let g = &mut ctx.accounts.guarantee;
        g.shares = g.shares.checked_sub(shares).ok_or(SasonaError::MoreThanIsThere)?;

        let exit = &mut ctx.accounts.exit;
        exit.owner = ctx.accounts.owner.key();
        exit.bump = ctx.bumps.exit;
        exit.shares = add(exit.shares, shares)?;
        exit.ready_at = Clock::get()?
            .unix_timestamp
            .checked_add(NOTICE_SECONDS)
            .ok_or(SasonaError::Overflow)?;

        emit!(ReleaseRequested { owner: exit.owner, shares: exit.shares, ready_at: exit.ready_at });
        Ok(())
    }

    /// Take back a guarantee whose notice has run out, as coins you hold.
    ///
    /// What comes back is the shares' part of the cover now, which is less
    /// than went in if claims were paid in the meantime.
    pub fn release(ctx: Context<Release>) -> Result<()> {
        require!(Clock::get()?.unix_timestamp >= ctx.accounts.exit.ready_at, SasonaError::NoticeNotOver);
        let shares = ctx.accounts.exit.shares;
        let coins = coins_for_shares(shares, ctx.accounts.cover.shares, ctx.accounts.cover.coins)?;

        let bump = [ctx.accounts.pool.bump];
        let seeds: &[&[u8]] = &[POOL_SEED, &bump];
        let signer: &[&[&[u8]]] = &[seeds];
        let a = &ctx.accounts;
        if coins > 0 {
            token::transfer(
                CpiContext::new(
                    a.token_program.key(),
                    Transfer {
                        from: a.cover_vault.to_account_info(),
                        to: a.owner_coin.to_account_info(),
                        authority: a.pool.to_account_info(),
                    },
                )
                .with_signer(signer),
                coins,
            )?;
        }
        // The last shares out are all the shares, and take every coin left.
        let cover = &mut ctx.accounts.cover;
        cover.shares -= shares;
        cover.coins -= coins;

        ctx.accounts.cover_vault.reload()?;
        let a = &ctx.accounts;
        a.cover.check(a.cover_vault.amount)?;
        emit!(Released { owner: a.owner.key(), shares, coins });
        Ok(())
    }
}

/// The checks and records every reading starts with, first or second.
fn start_reading(
    round: &Account<Round>,
    reading: &mut Account<Reading>,
    reader: Pubkey,
    bump: u8,
    endpoint_hash: [u8; 32],
    endpoint: String,
    question_hash: [u8; 32],
) -> Result<()> {
    // That the round is drawn was checked with the draw (check_drawn).
    require!(
        !endpoint.is_empty() && endpoint.len() <= MAX_ENDPOINT_LEN && endpoint.bytes().all(|b| (0x21..=0x7E).contains(&b)),
        SasonaError::BadEndpoint
    );
    require!(solana_sha256_hasher::hashv(&[endpoint.as_bytes()]).to_bytes() == endpoint_hash, SasonaError::BadEndpoint);
    reading.round = round.key();
    reading.endpoint = endpoint;
    reading.reader = reader;
    reading.question_hash = question_hash;
    reading.commit_slot = Clock::get()?.slot;
    reading.state = READING_COMMITTED;
    reading.bump = bump;
    emit!(ReadingCommitted { reading: reading.key(), round: reading.round, question_hash, commit_slot: reading.commit_slot });
    Ok(())
}

/// HMAC-SHA256 with a 32-byte key, over the concatenation of `parts`.
fn hmac_sha256(key: &[u8; 32], parts: &[&[u8]]) -> [u8; 32] {
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..32 {
        ipad[i] ^= key[i];
        opad[i] ^= key[i];
    }
    let mut inner: Vec<&[u8]> = Vec::with_capacity(parts.len() + 1);
    inner.push(&ipad);
    inner.extend_from_slice(parts);
    let inner = solana_sha256_hasher::hashv(&inner).to_bytes();
    solana_sha256_hasher::hashv(&[&opad, &inner]).to_bytes()
}

/// sasona-protocol SPEC.md 4.3: the membership drawn at `attempt` to read the
/// service whose hash is `endpoint_hash`, numbered from 1. `members` is at
/// least 1.
pub fn reader_number(final_seed: &[u8; 32], endpoint_hash: &[u8; 32], attempt: u32, members: u32) -> u32 {
    let digest = hmac_sha256(final_seed, &[b"reader", endpoint_hash, &attempt.to_be_bytes()]);
    let m = members as u64;
    let mut acc: u64 = 0;
    for byte in digest {
        acc = (acc * 256 + byte as u64) % m;
    }
    1 + acc as u32
}

/// Load a Seat from an account the caller passed, checking it is the seat
/// numbered `k`.
fn seat_at(info: &AccountInfo, k: u32) -> Result<Seat> {
    require_keys_eq!(*info.owner, crate::ID, SasonaError::NotTheSeat);
    let s = Seat::try_deserialize(&mut &info.try_borrow_data()?[..])?;
    let at = Pubkey::create_program_address(&[SEAT_SEED, &k.to_le_bytes(), &[s.bump]], &crate::ID)
        .map_err(|_| SasonaError::NotTheSeat)?;
    require_keys_eq!(info.key(), at, SasonaError::NotTheSeat);
    Ok(s)
}

fn store<T: AccountSerialize>(info: &AccountInfo, value: &T) -> Result<()> {
    let mut data = info.try_borrow_mut_data()?;
    let mut out: &mut [u8] = &mut data[..];
    value.try_serialize(&mut out)
}

/// Take `member` off the roster (SPEC.md 4.2). `accounts` are its seat, then,
/// unless it is the last seat, the last seat and the membership sitting in
/// it, which moves into the freed seat.
fn unseat(members: &mut Members, member: &mut Member, accounts: &[AccountInfo]) -> Result<()> {
    let (k, last) = (member.seat, members.seated);
    require!(k > 0 && k <= last, SasonaError::NotTheSeat);
    let seat_info = accounts.first().ok_or(SasonaError::NotTheSeat)?;
    let mut seat = seat_at(seat_info, k)?;
    require!(seat.member == member.number, SasonaError::NotTheSeat);
    if k == last {
        seat.member = 0;
        seat.owner = Pubkey::default();
    } else {
        let (last_info, mover_info) = match accounts {
            [_, l, m, ..] => (l, m),
            _ => return err!(SasonaError::NotTheSeat),
        };
        let mut last_seat = seat_at(last_info, last)?;
        require_keys_eq!(*mover_info.owner, crate::ID, SasonaError::NotTheSeat);
        let mut mover = Member::try_deserialize(&mut &mover_info.try_borrow_data()?[..])?;
        let at = Pubkey::create_program_address(&[MEMBER_SEED, &mover.number.to_le_bytes(), &[mover.bump]], &crate::ID)
            .map_err(|_| SasonaError::NotTheSeat)?;
        require_keys_eq!(mover_info.key(), at, SasonaError::NotTheSeat);
        require!(mover.number == last_seat.member && mover.seat == last, SasonaError::NotTheSeat);
        seat.member = last_seat.member;
        seat.owner = last_seat.owner;
        seat.since = Clock::get()?.slot;
        mover.seat = k;
        last_seat.member = 0;
        last_seat.owner = Pubkey::default();
        store(last_info, &last_seat)?;
        store(mover_info, &mover)?;
    }
    store(seat_info, &seat)?;
    member.seat = 0;
    members.seated = last - 1;
    Ok(())
}

/// Refuse unless `member`, held by `reader`, sits in `seat`, the seat drawn
/// for this service now (SPEC.md 4.3), and the round's window to read is
/// open. Seats drawn before it that exist now come in `skipped`, in order:
/// each must hold the first reading's reader, or a membership that sat down
/// after the round was committed. Those are the only reasons to pass one over.
#[allow(clippy::too_many_arguments)]
fn check_drawn(
    round: &Round,
    members: &Members,
    endpoint_hash: &[u8; 32],
    member: &Member,
    seat: &Seat,
    reader: Pubkey,
    skipped: &[AccountInfo],
    first_reader: Option<Pubkey>,
) -> Result<()> {
    require!(round.state == ROUND_DRAWN, SasonaError::RoundNotDrawn);
    let closes = round.entropy_slot.checked_add(READ_WINDOW_SLOTS).ok_or(SasonaError::Overflow)?;
    require!(Clock::get()?.slot <= closes, SasonaError::WindowClosed);
    require!(round.members > 0, SasonaError::NotDrawn);
    require_keys_eq!(member.owner, reader, SasonaError::NotTheReader);
    // Sitting in a seat is being active: every way out of active leaves the
    // seat (unseat), and a membership with no seat has no seat account to show.
    require!(seat.member == member.number, SasonaError::NotTheSeat);
    require!(seat.since < round.commit_slot, SasonaError::SatDownSince);
    let mut shown = 0usize;
    for attempt in 0..MAX_READER_ATTEMPTS {
        let k = reader_number(&round.final_seed, endpoint_hash, attempt, round.members);
        if k > members.seated {
            continue;
        }
        if k == member.seat {
            require!(shown == skipped.len(), SasonaError::NotDrawn);
            return Ok(());
        }
        let info = skipped.get(shown).ok_or(SasonaError::NotSkippable)?;
        shown += 1;
        let s = seat_at(info, k)?;
        require!(s.since >= round.commit_slot || Some(s.owner) == first_reader, SasonaError::NotSkippable);
    }
    err!(SasonaError::NotDrawn)
}

/// sasona-protocol SPEC.md 2.4: the verdict for a reply to the question built
/// from `nonce`.
pub fn verdict_of(reply: &[u8], nonce: &[u8; 16]) -> u8 {
    if reply.is_empty() {
        return 3;
    }
    let expected = expected_answer(nonce);
    if reply.windows(16).any(|w| w == expected) { 1 } else { 2 }
}

/// SPEC.md 2.1: the first 16 hex characters of sha256 of the nonce in hex.
pub fn expected_answer(nonce: &[u8; 16]) -> [u8; 16] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut n = [0u8; 32];
    for (i, b) in nonce.iter().enumerate() {
        n[2 * i] = HEX[(b >> 4) as usize];
        n[2 * i + 1] = HEX[(b & 15) as usize];
    }
    let digest = solana_sha256_hasher::hashv(&[&n]).to_bytes();
    let mut expect = [0u8; 16];
    for (i, b) in digest[..8].iter().enumerate() {
        expect[2 * i] = HEX[(b >> 4) as usize];
        expect[2 * i + 1] = HEX[(b & 15) as usize];
    }
    expect
}

/// sasona-protocol SPEC.md 3.3: what a pair settles, from its two verdicts.
pub fn pair_outcome(first: u8, second: u8) -> u8 {
    match (first, second) {
        (_, 1) => PAIR_WORKS_NOW,
        (1, _) => PAIR_FALSE_OR_DECAYED,
        _ => PAIR_AGREED_FAILS,
    }
}

/// Buy coin from the pool with `dollars` that have already arrived in its
/// dollar account, burn the `burn` dollars' share of it, and send the rest
/// to the participants. Returns (coins burned, coins shared).
///
/// The price follows the pool's recorded reserves, constant product, rounded
/// in the pool's favour. The product of the reserves may only grow.
fn buy_and_share<'info>(
    pool: &mut Account<'info, Pool>,
    token_program: &Program<'info, Token>,
    coin_mint: &Account<'info, Mint>,
    pool_coin: &Account<'info, TokenAccount>,
    network: &Account<'info, TokenAccount>,
    dollars: u64,
    burn: u64,
) -> Result<(u64, u64)> {
    let (usd, coins) = (pool.usd_reserve, pool.coin_reserve);
    let out = coins_out(dollars, usd, coins)?;
    require!(out > 0, SasonaError::TooSmall);
    // Rounded up, like the burn itself: the burn is never less than its share.
    let burned = u64::try_from((out as u128 * burn as u128).div_ceil(dollars as u128))
        .map_err(|_| SasonaError::Overflow)?;
    let shared = out - burned;

    pool.usd_reserve = add(usd, dollars)?;
    pool.coin_reserve = coins - out;
    pool.outside = add(pool.outside, shared)?;
    require!(
        pool.usd_reserve as u128 * pool.coin_reserve as u128 >= usd as u128 * coins as u128,
        SasonaError::PriceMoved
    );

    let bump = [pool.bump];
    let seeds: &[&[u8]] = &[POOL_SEED, &bump];
    let signer: &[&[&[u8]]] = &[seeds];
    if burned > 0 {
        token::burn(
            CpiContext::new(
                token_program.key(),
                Burn {
                    mint: coin_mint.to_account_info(),
                    from: pool_coin.to_account_info(),
                    authority: pool.to_account_info(),
                },
            )
            .with_signer(signer),
            burned,
        )?;
    }
    if shared > 0 {
        // Every participant seat is empty for now, so all of it is the
        // network's. See the note on DEVELOPER_POINTS.
        token::transfer(
            CpiContext::new(
                token_program.key(),
                Transfer {
                    from: pool_coin.to_account_info(),
                    to: network.to_account_info(),
                    authority: pool.to_account_info(),
                },
            )
            .with_signer(signer),
            shared,
        )?;
    }
    Ok((burned, shared))
}

// ------------------------------------------------------------------ accounts

#[derive(Accounts)]
pub struct Open<'info> {
    #[account(mut)]
    pub depositor: Signer<'info>,

    #[account(init, payer = depositor, space = 8 + Pool::INIT_SPACE,
              seeds = [POOL_SEED], bump)]
    pub pool: Account<'info, Pool>,

    /// The coin. Created here with the pool as its only authority and no
    /// freeze authority, so there is never a moment when a key can mint it.
    #[account(init, payer = depositor, seeds = [COIN_SEED], bump,
              mint::decimals = COIN_DECIMALS, mint::authority = pool)]
    pub coin_mint: Account<'info, Mint>,

    #[account(address = USD_MINT @ SasonaError::NotTheDollar, mint::decimals = USD_DECIMALS)]
    pub usd_mint: Account<'info, Mint>,

    /// The pool's own token accounts are created here at fixed addresses, so
    /// no later instruction can be handed a look-alike.
    #[account(init, payer = depositor, seeds = [POOL_USD_SEED], bump,
              token::mint = usd_mint, token::authority = pool)]
    pub pool_usd: Account<'info, TokenAccount>,

    #[account(init, payer = depositor, seeds = [POOL_COIN_SEED], bump,
              token::mint = coin_mint, token::authority = pool)]
    pub pool_coin: Account<'info, TokenAccount>,

    /// Entry fees wait here until the fee split exists.
    #[account(init, payer = depositor, seeds = [FEES_SEED], bump,
              token::mint = usd_mint, token::authority = pool)]
    pub fees: Account<'info, TokenAccount>,

    #[account(mut, token::mint = usd_mint, token::authority = depositor)]
    pub depositor_usd: Account<'info, TokenAccount>,

    #[account(init, payer = depositor,
              associated_token::mint = coin_mint, associated_token::authority = depositor)]
    pub depositor_coin: Account<'info, TokenAccount>,

    #[account(init, payer = depositor, space = 8 + Cover::INIT_SPACE, seeds = [COVER_SEED], bump)]
    pub cover: Account<'info, Cover>,

    /// Held by the pool, not by any depositor, so no depositor can move it.
    #[account(init, payer = depositor, seeds = [COVER_VAULT_SEED], bump,
              token::mint = coin_mint, token::authority = pool)]
    pub cover_vault: Account<'info, TokenAccount>,

    #[account(init, payer = depositor, space = 8 + Guarantee::INIT_SPACE,
              seeds = [GUARANTEE_SEED, depositor.key().as_ref()], bump)]
    pub guarantee: Account<'info, Guarantee>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

impl<'info> Open<'info> {
    fn cpi_transfer(
        &self,
        from: &Account<'info, TokenAccount>,
        to: &Account<'info, TokenAccount>,
        authority: AccountInfo<'info>,
    ) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        CpiContext::new(
            self.token_program.key(),
            Transfer { from: from.to_account_info(), to: to.to_account_info(), authority },
        )
    }

    fn cpi_mint(&self, to: &Account<'info, TokenAccount>) -> CpiContext<'_, '_, '_, 'info, MintTo<'info>> {
        CpiContext::new(
            self.token_program.key(),
            MintTo {
                mint: self.coin_mint.to_account_info(),
                to: to.to_account_info(),
                authority: self.pool.to_account_info(),
            },
        )
    }
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub depositor: Signer<'info>,

    #[account(mut, seeds = [POOL_SEED], bump = pool.bump)]
    pub pool: Account<'info, Pool>,

    #[account(mut, address = pool.coin_mint)]
    pub coin_mint: Account<'info, Mint>,

    #[account(address = pool.usd_mint @ SasonaError::NotTheDollar)]
    pub usd_mint: Account<'info, Mint>,

    #[account(mut, seeds = [POOL_USD_SEED], bump)]
    pub pool_usd: Account<'info, TokenAccount>,

    #[account(mut, seeds = [POOL_COIN_SEED], bump)]
    pub pool_coin: Account<'info, TokenAccount>,

    #[account(mut, seeds = [FEES_SEED], bump)]
    pub fees: Account<'info, TokenAccount>,

    #[account(mut, token::mint = usd_mint, token::authority = depositor)]
    pub depositor_usd: Account<'info, TokenAccount>,

    /// These two already exist if this depositor has deposited before, and
    /// then the constraints are checked against what is there.
    #[account(init_if_needed, payer = depositor,
              associated_token::mint = coin_mint, associated_token::authority = depositor)]
    pub depositor_coin: Account<'info, TokenAccount>,

    #[account(mut, seeds = [COVER_SEED], bump = cover.bump)]
    pub cover: Account<'info, Cover>,

    #[account(mut, seeds = [COVER_VAULT_SEED], bump)]
    pub cover_vault: Account<'info, TokenAccount>,

    /// CHECK: where this depositor's guarantee lived before the cover
    /// existed. Must be empty: a guarantee still there has to join the cover
    /// first, or its old count would be mixed with shares.
    #[account(seeds = [VAULT_SEED, depositor.key().as_ref()], bump)]
    pub legacy_vault: UncheckedAccount<'info>,

    #[account(init_if_needed, payer = depositor, space = 8 + Guarantee::INIT_SPACE,
              seeds = [GUARANTEE_SEED, depositor.key().as_ref()], bump)]
    pub guarantee: Account<'info, Guarantee>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

impl<'info> Deposit<'info> {
    fn cpi_transfer(
        &self,
        from: &Account<'info, TokenAccount>,
        to: &Account<'info, TokenAccount>,
        authority: AccountInfo<'info>,
    ) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        CpiContext::new(
            self.token_program.key(),
            Transfer { from: from.to_account_info(), to: to.to_account_info(), authority },
        )
    }

    fn cpi_mint(&self, to: &Account<'info, TokenAccount>) -> CpiContext<'_, '_, '_, 'info, MintTo<'info>> {
        CpiContext::new(
            self.token_program.key(),
            MintTo {
                mint: self.coin_mint.to_account_info(),
                to: to.to_account_info(),
                authority: self.pool.to_account_info(),
            },
        )
    }
}

#[derive(Accounts)]
pub struct PayFee<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(mut, seeds = [POOL_SEED], bump = pool.bump)]
    pub pool: Account<'info, Pool>,

    #[account(mut, address = pool.coin_mint)]
    pub coin_mint: Account<'info, Mint>,

    #[account(address = pool.usd_mint @ SasonaError::NotTheDollar)]
    pub usd_mint: Account<'info, Mint>,

    #[account(mut, seeds = [POOL_USD_SEED], bump)]
    pub pool_usd: Account<'info, TokenAccount>,

    #[account(mut, seeds = [POOL_COIN_SEED], bump)]
    pub pool_coin: Account<'info, TokenAccount>,

    #[account(mut, token::mint = usd_mint, token::authority = payer)]
    pub payer_usd: Account<'info, TokenAccount>,

    /// Read only, so the end-of-instruction check covers every account.
    #[account(seeds = [FEES_SEED], bump)]
    pub fees: Account<'info, TokenAccount>,

    /// The network's coin. Held by the pool, with no instruction that moves
    /// it out yet; see the note on DEVELOPER_POINTS.
    #[account(init_if_needed, payer = payer, seeds = [NETWORK_SEED], bump,
              token::mint = coin_mint, token::authority = pool)]
    pub network: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SettleEntryFees<'info> {
    #[account(mut)]
    pub caller: Signer<'info>,

    #[account(mut, seeds = [POOL_SEED], bump = pool.bump)]
    pub pool: Account<'info, Pool>,

    #[account(mut, address = pool.coin_mint)]
    pub coin_mint: Account<'info, Mint>,

    #[account(mut, seeds = [POOL_USD_SEED], bump)]
    pub pool_usd: Account<'info, TokenAccount>,

    #[account(mut, seeds = [POOL_COIN_SEED], bump)]
    pub pool_coin: Account<'info, TokenAccount>,

    #[account(mut, seeds = [FEES_SEED], bump)]
    pub fees: Account<'info, TokenAccount>,

    #[account(init_if_needed, payer = caller, seeds = [NETWORK_SEED], bump,
              token::mint = coin_mint, token::authority = pool)]
    pub network: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct AddDepth<'info> {
    pub giver: Signer<'info>,

    #[account(mut, seeds = [POOL_SEED], bump = pool.bump)]
    pub pool: Account<'info, Pool>,

    /// Read only: there is no coin account here at all, so this instruction
    /// cannot move a coin even if its arithmetic were wrong.
    #[account(address = pool.coin_mint)]
    pub coin_mint: Account<'info, Mint>,

    #[account(address = pool.usd_mint @ SasonaError::NotTheDollar)]
    pub usd_mint: Account<'info, Mint>,

    #[account(mut, seeds = [POOL_USD_SEED], bump)]
    pub pool_usd: Account<'info, TokenAccount>,

    #[account(seeds = [FEES_SEED], bump)]
    pub fees: Account<'info, TokenAccount>,

    #[account(mut, token::mint = usd_mint, token::authority = giver)]
    pub giver_usd: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
#[instruction(pool_fingerprint: [u8; 32])]
pub struct OpenRound<'info> {
    #[account(mut)]
    pub opener: Signer<'info>,

    #[account(init, payer = opener, space = 8 + Round::INIT_SPACE,
              seeds = [ROUND_SEED, pool_fingerprint.as_ref()], bump)]
    pub round: Account<'info, Round>,

    /// How many memberships there are now. Created here if nobody has
    /// joined yet, so a round can be opened before anyone has.
    #[account(init_if_needed, payer = opener, space = 8 + Members::INIT_SPACE, seeds = [MEMBERS_SEED], bump)]
    pub members: Account<'info, Members>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RevealRound<'info> {
    #[account(mut, seeds = [ROUND_SEED, round.pool_fingerprint.as_ref()], bump = round.bump)]
    pub round: Account<'info, Round>,

    /// CHECK: the round's opener, who gets the bond back.
    #[account(mut, address = round.opener)]
    pub opener: UncheckedAccount<'info>,

    /// CHECK: the SlotHashes sysvar, by address; read as raw bytes.
    #[account(address = SLOT_HASHES_ID)]
    pub slot_hashes: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct MarkWithheld<'info> {
    #[account(mut, seeds = [ROUND_SEED, round.pool_fingerprint.as_ref()], bump = round.bump)]
    pub round: Account<'info, Round>,

    /// CHECK: the SlotHashes sysvar, by address; read as raw bytes.
    #[account(address = SLOT_HASHES_ID)]
    pub slot_hashes: UncheckedAccount<'info>,
}

#[derive(Accounts)]
#[instruction(endpoint_hash: [u8; 32])]
pub struct CommitReading<'info> {
    #[account(mut)]
    pub reader: Signer<'info>,

    #[account(seeds = [ROUND_SEED, round.pool_fingerprint.as_ref()], bump = round.bump)]
    pub round: Account<'info, Round>,

    #[account(seeds = [MEMBERS_SEED], bump = members.bump)]
    pub members: Account<'info, Members>,

    /// The reader's membership, the one drawn for this service.
    #[account(seeds = [MEMBER_SEED, member.number.to_le_bytes().as_ref()], bump = member.bump)]
    pub member: Account<'info, Member>,

    /// The seat it sits in.
    #[account(seeds = [SEAT_SEED, member.seat.to_le_bytes().as_ref()], bump = seat.bump)]
    pub seat: Account<'info, Seat>,

    #[account(init, payer = reader, space = 8 + Reading::INIT_SPACE,
              seeds = [READING_SEED, round.key().as_ref(), endpoint_hash.as_ref()], bump)]
    pub reading: Account<'info, Reading>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(nonce: [u8; 16])]
pub struct RevealReading<'info> {
    #[account(mut)]
    pub reader: Signer<'info>,

    #[account(mut, has_one = reader @ SasonaError::NotTheReader)]
    pub reading: Account<'info, Reading>,

    /// Which reading a nonce belongs to: the earliest committed that has
    /// revealed it.
    #[account(init_if_needed, payer = reader, space = 8 + UsedNonce::INIT_SPACE,
              seeds = [NONCE_SEED, nonce.as_ref()], bump)]
    pub used_nonce: Account<'info, UsedNonce>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(endpoint_hash: [u8; 32])]
pub struct CommitSecondReading<'info> {
    #[account(mut)]
    pub reader: Signer<'info>,

    /// The re-read round.
    #[account(seeds = [ROUND_SEED, round.pool_fingerprint.as_ref()], bump = round.bump)]
    pub round: Account<'info, Round>,

    #[account(seeds = [MEMBERS_SEED], bump = members.bump)]
    pub members: Account<'info, Members>,

    /// The reader's membership, the one drawn for this service.
    #[account(seeds = [MEMBER_SEED, member.number.to_le_bytes().as_ref()], bump = member.bump)]
    pub member: Account<'info, Member>,

    /// The seat it sits in.
    #[account(seeds = [SEAT_SEED, member.seat.to_le_bytes().as_ref()], bump = seat.bump)]
    pub seat: Account<'info, Seat>,

    #[account(init, payer = reader, space = 8 + Reading::INIT_SPACE,
              seeds = [READING_SEED, round.key().as_ref(), endpoint_hash.as_ref()], bump)]
    pub reading: Account<'info, Reading>,

    /// The reading being re-tested.
    pub first: Account<'info, Reading>,

    #[account(init, payer = reader, space = 8 + Pair::INIT_SPACE,
              seeds = [PAIR_SEED, reading.key().as_ref()], bump)]
    pub pair: Account<'info, Pair>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SettlePair<'info> {
    #[account(mut, seeds = [PAIR_SEED, second.key().as_ref()], bump = pair.bump,
              has_one = first, has_one = second)]
    pub pair: Account<'info, Pair>,
    pub first: Account<'info, Reading>,
    pub second: Account<'info, Reading>,
}

#[derive(Accounts)]
pub struct MarkLapsed<'info> {
    #[account(mut)]
    pub reading: Account<'info, Reading>,
}

#[derive(Accounts)]
pub struct JoinCover<'info> {
    #[account(mut)]
    pub caller: Signer<'info>,

    #[account(seeds = [POOL_SEED], bump = pool.bump)]
    pub pool: Account<'info, Pool>,

    #[account(address = pool.coin_mint)]
    pub coin_mint: Account<'info, Mint>,

    #[account(init_if_needed, payer = caller, space = 8 + Cover::INIT_SPACE, seeds = [COVER_SEED], bump)]
    pub cover: Account<'info, Cover>,

    #[account(init_if_needed, payer = caller, seeds = [COVER_VAULT_SEED], bump,
              token::mint = coin_mint, token::authority = pool)]
    pub cover_vault: Account<'info, TokenAccount>,

    /// CHECK: the guarantee's owner, named by the guarantee record; receives
    /// the old vault's rent.
    #[account(mut, address = guarantee.owner)]
    pub owner: UncheckedAccount<'info>,

    #[account(mut, seeds = [GUARANTEE_SEED, guarantee.owner.as_ref()], bump = guarantee.bump)]
    pub guarantee: Account<'info, Guarantee>,

    #[account(mut, seeds = [VAULT_SEED, guarantee.owner.as_ref()], bump,
              token::mint = coin_mint, token::authority = pool)]
    pub legacy_vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Claim<'info> {
    pub judge: Signer<'info>,

    #[account(mut, seeds = [POOL_SEED], bump = pool.bump)]
    pub pool: Account<'info, Pool>,

    #[account(mut, address = pool.coin_mint)]
    pub coin_mint: Account<'info, Mint>,

    #[account(address = pool.usd_mint @ SasonaError::NotTheDollar)]
    pub usd_mint: Account<'info, Mint>,

    #[account(mut, seeds = [POOL_USD_SEED], bump)]
    pub pool_usd: Account<'info, TokenAccount>,

    #[account(mut, seeds = [POOL_COIN_SEED], bump)]
    pub pool_coin: Account<'info, TokenAccount>,

    #[account(seeds = [FEES_SEED], bump)]
    pub fees: Account<'info, TokenAccount>,

    #[account(mut, seeds = [COVER_SEED], bump = cover.bump)]
    pub cover: Account<'info, Cover>,

    #[account(mut, seeds = [COVER_VAULT_SEED], bump)]
    pub cover_vault: Account<'info, TokenAccount>,

    /// The buyer being paid back, in the pool's dollar. Never one of the
    /// pool's own accounts, where the dollars would sit unrecorded.
    #[account(mut, token::mint = usd_mint,
              constraint = claimant_usd.key() != pool_usd.key() @ SasonaError::NotTheClaimant,
              constraint = claimant_usd.key() != fees.key() @ SasonaError::NotTheClaimant)]
    pub claimant_usd: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct RequestRelease<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(mut, seeds = [GUARANTEE_SEED, owner.key().as_ref()], bump = guarantee.bump,
              has_one = owner)]
    pub guarantee: Account<'info, Guarantee>,

    #[account(init_if_needed, payer = owner, space = 8 + Exit::INIT_SPACE,
              seeds = [EXIT_SEED, owner.key().as_ref()], bump)]
    pub exit: Account<'info, Exit>,

    /// CHECK: must be empty; see `Deposit::legacy_vault`.
    #[account(seeds = [VAULT_SEED, owner.key().as_ref()], bump)]
    pub legacy_vault: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Release<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(seeds = [POOL_SEED], bump = pool.bump)]
    pub pool: Account<'info, Pool>,

    #[account(address = pool.coin_mint)]
    pub coin_mint: Account<'info, Mint>,

    #[account(mut, seeds = [COVER_SEED], bump = cover.bump)]
    pub cover: Account<'info, Cover>,

    #[account(mut, seeds = [COVER_VAULT_SEED], bump)]
    pub cover_vault: Account<'info, TokenAccount>,

    #[account(mut, seeds = [EXIT_SEED, owner.key().as_ref()], bump = exit.bump,
              has_one = owner, close = owner)]
    pub exit: Account<'info, Exit>,

    #[account(init_if_needed, payer = owner,
              associated_token::mint = coin_mint, associated_token::authority = owner)]
    pub owner_coin: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct JoinMembers<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(seeds = [POOL_SEED], bump = pool.bump)]
    pub pool: Account<'info, Pool>,

    #[account(address = pool.coin_mint)]
    pub coin_mint: Account<'info, Mint>,

    #[account(mut, token::mint = coin_mint, token::authority = owner)]
    pub owner_coin: Account<'info, TokenAccount>,

    #[account(init_if_needed, payer = owner, space = 8 + Members::INIT_SPACE, seeds = [MEMBERS_SEED], bump)]
    pub members: Account<'info, Members>,

    #[account(init, payer = owner, space = 8 + Member::INIT_SPACE,
              seeds = [MEMBER_SEED, (members.count + 1).to_le_bytes().as_ref()], bump)]
    pub member: Account<'info, Member>,

    /// The seat after the last. It may exist already, empty, from when the
    /// roster was longer.
    #[account(init_if_needed, payer = owner, space = 8 + Seat::INIT_SPACE,
              seeds = [SEAT_SEED, (members.seated + 1).to_le_bytes().as_ref()], bump)]
    pub seat: Account<'info, Seat>,

    #[account(init_if_needed, payer = owner, seeds = [STAKES_SEED], bump,
              token::mint = coin_mint, token::authority = pool)]
    pub stakes: Account<'info, TokenAccount>,

    /// Where taken stakes wait. Made here so that taking one never has to.
    #[account(init_if_needed, payer = owner, seeds = [HELD_SEED], bump,
              token::mint = coin_mint, token::authority = pool)]
    pub held: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct AskToLeave<'info> {
    pub owner: Signer<'info>,

    #[account(mut, seeds = [MEMBERS_SEED], bump = members.bump)]
    pub members: Account<'info, Members>,

    #[account(mut, seeds = [MEMBER_SEED, member.number.to_le_bytes().as_ref()], bump = member.bump,
              has_one = owner @ SasonaError::NotTheOwner)]
    pub member: Account<'info, Member>,
}

#[derive(Accounts)]
pub struct Leave<'info> {
    pub owner: Signer<'info>,

    #[account(seeds = [POOL_SEED], bump = pool.bump)]
    pub pool: Account<'info, Pool>,

    #[account(address = pool.coin_mint)]
    pub coin_mint: Account<'info, Mint>,

    #[account(mut, seeds = [MEMBER_SEED, member.number.to_le_bytes().as_ref()], bump = member.bump,
              has_one = owner @ SasonaError::NotTheOwner)]
    pub member: Account<'info, Member>,

    #[account(mut, seeds = [STAKES_SEED], bump)]
    pub stakes: Account<'info, TokenAccount>,

    #[account(mut, token::mint = coin_mint, token::authority = owner)]
    pub owner_coin: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct ChallengeReading<'info> {
    #[account(mut)]
    pub challenger: Signer<'info>,

    pub reading: Account<'info, Reading>,

    #[account(mut, seeds = [MEMBER_SEED, reading.member.to_le_bytes().as_ref()], bump = member.bump)]
    pub member: Account<'info, Member>,

    #[account(init, payer = challenger, space = 8 + Challenge::INIT_SPACE,
              seeds = [CHALLENGE_SEED, reading.key().as_ref()], bump)]
    pub challenge: Account<'info, Challenge>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(len: u32)]
pub struct OpenEvidence<'info> {
    #[account(mut)]
    pub reader: Signer<'info>,

    #[account(has_one = reader @ SasonaError::NotTheReader)]
    pub reading: Account<'info, Reading>,

    #[account(init, payer = reader, space = 8 + 32 + 1 + 4 + len as usize,
              seeds = [EVIDENCE_SEED, reading.key().as_ref()], bump)]
    pub evidence: Account<'info, Evidence>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WriteEvidence<'info> {
    pub reader: Signer<'info>,

    #[account(has_one = reader @ SasonaError::NotTheReader)]
    pub reading: Account<'info, Reading>,

    #[account(mut, seeds = [EVIDENCE_SEED, reading.key().as_ref()], bump, has_one = reading)]
    pub evidence: Account<'info, Evidence>,
}

#[derive(Accounts)]
#[instruction(nonce: [u8; 16])]
pub struct AnswerChallenge<'info> {
    #[account(mut, seeds = [CHALLENGE_SEED, reading.key().as_ref()], bump = challenge.bump, has_one = reading)]
    pub challenge: Account<'info, Challenge>,

    pub reading: Account<'info, Reading>,

    #[account(mut, seeds = [EVIDENCE_SEED, reading.key().as_ref()], bump, has_one = reading)]
    pub evidence: Account<'info, Evidence>,

    #[account(seeds = [NONCE_SEED, nonce.as_ref()], bump)]
    pub used_nonce: Account<'info, UsedNonce>,

    #[account(mut, seeds = [MEMBER_SEED, reading.member.to_le_bytes().as_ref()], bump = member.bump)]
    pub member: Account<'info, Member>,

    /// CHECK: the reader, who gets the bond. Only lamports move.
    #[account(mut, address = reading.reader @ SasonaError::NotTheReader)]
    pub reader: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct UpholdChallenge<'info> {
    #[account(mut, seeds = [CHALLENGE_SEED, reading.key().as_ref()], bump = challenge.bump, has_one = reading)]
    pub challenge: Account<'info, Challenge>,

    #[account(mut)]
    pub reading: Account<'info, Reading>,

    #[account(mut, seeds = [MEMBERS_SEED], bump = members.bump)]
    pub members: Account<'info, Members>,

    #[account(mut, seeds = [MEMBER_SEED, reading.member.to_le_bytes().as_ref()], bump = member.bump)]
    pub member: Account<'info, Member>,

    #[account(seeds = [POOL_SEED], bump = pool.bump)]
    pub pool: Account<'info, Pool>,

    #[account(address = pool.coin_mint)]
    pub coin_mint: Account<'info, Mint>,

    #[account(mut, seeds = [STAKES_SEED], bump)]
    pub stakes: Account<'info, TokenAccount>,

    #[account(mut, seeds = [HELD_SEED], bump)]
    pub held: Account<'info, TokenAccount>,

    /// CHECK: the challenger, who gets the bond back. Only lamports move.
    #[account(mut, address = challenge.challenger @ SasonaError::NotTheChallenger)]
    pub challenger: UncheckedAccount<'info>,

    #[account(mut, token::mint = coin_mint, token::authority = challenge.challenger)]
    pub challenger_coin: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct SetQuote<'info> {
    #[account(mut)]
    pub reader: Signer<'info>,

    #[account(has_one = reader @ SasonaError::NotTheReader)]
    pub reading: Account<'info, Reading>,

    #[account(seeds = [MEMBER_SEED, reading.member.to_le_bytes().as_ref()], bump = member.bump)]
    pub member: Account<'info, Member>,

    #[account(init_if_needed, payer = reader, space = 8 + Quote::INIT_SPACE,
              seeds = [QUOTE_SEED, reading.key().as_ref()], bump)]
    pub quote: Account<'info, Quote>,

    pub system_program: Program<'info, System>,
}

// --------------------------------------------------------------------- state

#[account]
#[derive(InitSpace)]
pub struct Pool {
    pub bump: u8,
    pub usd_mint: Pubkey,
    pub coin_mint: Pubkey,
    pub usd_reserve: u64,
    pub coin_reserve: u64,
    /// Coins that exist outside the pool: held by depositors or locked in
    /// guarantees.
    pub outside: u64,
    /// Entry fees collected and not yet paid out.
    pub fees_held: u64,
}

impl Pool {
    /// Checked at the end of every instruction, against balances read back
    /// from the token program after the transfers have happened.
    pub fn check(&self, coin_supply: u64, pool_usd: u64, fees: u64) -> Result<()> {
        require!(add(self.coin_reserve, self.outside)? == coin_supply, SasonaError::SupplyMismatch);
        require!(pool_usd >= self.usd_reserve, SasonaError::DollarsMissing);
        require!(fees >= self.fees_held, SasonaError::DollarsMissing);
        Ok(())
    }

    /// After a deposit the price may not fall, and may rise by less than one
    /// coin unit's worth. With C/U before and C'/U' after, that is
    /// 0 <= C*U' - C'*U < U.
    pub fn price_held(&self, usd_before: u64, coin_before: u64) -> Result<()> {
        let before = coin_before as u128 * self.usd_reserve as u128;
        let after = self.coin_reserve as u128 * usd_before as u128;
        require!(before >= after && before - after < usd_before as u128, SasonaError::PriceMoved);
        Ok(())
    }
}

#[account]
#[derive(InitSpace)]
pub struct Guarantee {
    pub owner: Pubkey,
    /// Shares of the cover. Before the cover existed this field counted
    /// coins in the depositor's own vault; `join_cover` converts it.
    pub shares: u64,
    pub bump: u8,
}

/// The guarantees, together. Every share is a claim on the same coins.
#[account]
#[derive(InitSpace)]
pub struct Cover {
    pub bump: u8,
    pub shares: u64,
    /// Coins in the cover vault, as recorded. Coins sent to the vault by
    /// anyone else are ignored, so nobody can change what a share is worth by
    /// giving.
    pub coins: u64,
}

impl Cover {
    /// Add guarantee coins that have just arrived in the vault, and return
    /// the shares they buy, rounded down so existing shares never lose.
    pub fn take_in(&mut self, coins: u64) -> Result<u64> {
        let shares = shares_for(coins, self.shares, self.coins)?;
        require!(shares > 0, SasonaError::TooSmall);
        self.shares = add(self.shares, shares)?;
        self.coins = add(self.coins, coins)?;
        Ok(shares)
    }

    pub fn check(&self, vault: u64) -> Result<()> {
        require!(vault >= self.coins, SasonaError::SupplyMismatch);
        require!((self.shares == 0) == (self.coins == 0), SasonaError::SupplyMismatch);
        Ok(())
    }
}

/// A draw's commitment, and once revealed, its seed and the entropy it was
/// mixed with. Kept for good, so anyone can re-run the draw.
#[account]
#[derive(InitSpace)]
pub struct Round {
    pub opener: Pubkey,
    pub pool_fingerprint: [u8; 32],
    pub pool_size: u32,
    pub count: u16,
    pub seed_hash: [u8; 32],
    pub commit_slot: u64,
    pub state: u8,
    pub seed: [u8; 32],
    pub entropy_slot: u64,
    pub entropy: [u8; 32],
    pub final_seed: [u8; 32],
    pub bump: u8,
    /// How many memberships there were when the round was committed: its
    /// roster is memberships 1 to this (SPEC.md 4.2).
    pub members: u32,
}

/// One test of one drawn service: the question's hash before the call, and
/// after it the reply's hash and the verdict.
#[account]
#[derive(InitSpace)]
pub struct Reading {
    pub round: Pubkey,
    #[max_len(256)]
    pub endpoint: String,
    pub reader: Pubkey,
    pub question_hash: [u8; 32],
    pub commit_slot: u64,
    pub state: u8,
    pub reply_hash: [u8; 32],
    pub verdict: u8,
    pub reveal_slot: u64,
    pub bump: u8,
    /// The membership that took it, or 0 for a reading taken before members.
    pub member: u32,
    /// When it was revealed, for the 30 days it can be challenged.
    pub reveal_time: i64,
}

/// How many memberships have ever been taken, and how many seats the roster
/// has now (SPEC.md 4.2).
#[account]
#[derive(InitSpace)]
pub struct Members {
    pub count: u32,
    pub seated: u32,
    pub bump: u8,
}

/// One membership: one stake, held by `owner` (SPEC.md 4.1). `seat` is 0
/// once it has left the roster.
#[account]
#[derive(InitSpace)]
pub struct Member {
    pub owner: Pubkey,
    pub number: u32,
    pub seat: u32,
    pub state: u8,
    pub stake: u64,
    pub leave_at: i64,
    pub open_challenges: u32,
    pub bump: u8,
}

/// One seat of the roster, and the membership in it; `member` is 0 while the
/// seat is past the end of the roster.
#[account]
#[derive(InitSpace)]
pub struct Seat {
    pub member: u32,
    pub owner: Pubkey,
    /// The slot the membership sat down here, by joining or moving up. It
    /// reads only for rounds committed after it (SPEC.md 4.2).
    pub since: u64,
    pub bump: u8,
}

/// A challenge to one reading (SPEC.md section 5).
#[account]
#[derive(InitSpace)]
pub struct Challenge {
    pub reading: Pubkey,
    pub challenger: Pubkey,
    pub deadline: i64,
    pub state: u8,
    pub bump: u8,
}

/// The quote on one reading (SPEC.md 6.1). `rate` is 0 once withdrawn.
#[account]
#[derive(InitSpace)]
pub struct Quote {
    pub reading: Pubkey,
    pub member: u32,
    pub rate: u16,
    pub set_slot: u64,
    pub set_time: i64,
    pub bump: u8,
}

/// A reading's reply, put on chain by its reader. Sealed once an answer holds.
#[account]
pub struct Evidence {
    pub reading: Pubkey,
    pub sealed: bool,
    pub reply: Vec<u8>,
}

/// A second reading and the first one it re-tests. The outcome is 0 until
/// the second is revealed and the pair settled.
#[account]
#[derive(InitSpace)]
pub struct Pair {
    pub first: Pubkey,
    pub second: Pubkey,
    pub outcome: u8,
    pub bump: u8,
}

/// A revealed nonce, and the reading it belongs to: the earliest committed of
/// those that revealed it. A verifier counts a reading only if its nonce's
/// record names it.
#[account]
#[derive(InitSpace)]
pub struct UsedNonce {
    pub reading: Pubkey,
    pub commit_slot: u64,
}

/// SPEC.md 2.1 to 2.3: the canonical question for a nonce, as bytes. The
/// nonce is written as 32 lowercase hex characters.
pub fn canonical_question(nonce: &[u8; 16]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut n = [0u8; 32];
    for (i, b) in nonce.iter().enumerate() {
        n[2 * i] = HEX[(b >> 4) as usize];
        n[2 * i + 1] = HEX[(b & 15) as usize];
    }
    let digest = solana_sha256_hasher::hashv(&[&n]).to_bytes();
    let mut expect = [0u8; 16];
    for (i, b) in digest[..8].iter().enumerate() {
        expect[2 * i] = HEX[(b >> 4) as usize];
        expect[2 * i + 1] = HEX[(b & 15) as usize];
    }
    // The code as a JSON string: its quotes escaped, its newline as \n.
    let mut code = Vec::with_capacity(96);
    code.extend_from_slice(b"\"import hashlib\\nprint(hashlib.sha256(\\\"");
    code.extend_from_slice(&n);
    code.extend_from_slice(b"\\\".encode()).hexdigest()[:16])\"");

    let mut q = Vec::with_capacity(512);
    q.extend_from_slice(b"{\"body\":{\"code\":");
    q.extend_from_slice(&code);
    q.extend_from_slice(b",\"language\":\"python\"},\"capability\":\"execute\",\"code\":");
    q.extend_from_slice(&code);
    q.extend_from_slice(b",\"command\":[\"python\",\"-c\",");
    q.extend_from_slice(&code);
    q.extend_from_slice(b"],\"expect\":\"");
    q.extend_from_slice(&expect);
    q.extend_from_slice(b"\",\"nonce\":\"");
    q.extend_from_slice(&n);
    q.extend_from_slice(b"\",\"tier\":1}");
    q
}

pub enum Entropy {
    Found(u64, [u8; 32]),
    TooEarly,
    Gone,
}

/// Find the hash of the earliest slot at or after `target` in the raw
/// SlotHashes data: a u64 count, then (u64 slot, 32-byte hash) entries,
/// newest first. `Gone` once the oldest entry is later than `target`, which
/// is when the window to reveal has closed.
pub fn entropy_for(data: &[u8], target: u64) -> Result<Entropy> {
    require!(data.len() >= 8, SasonaError::BadSlotHashes);
    let n = u64::from_le_bytes(data[..8].try_into().unwrap()) as usize;
    require!(n > 0 && data.len() >= 8 + n * 40, SasonaError::BadSlotHashes);
    let entry = |i: usize| {
        let at = 8 + i * 40;
        let slot = u64::from_le_bytes(data[at..at + 8].try_into().unwrap());
        let hash: [u8; 32] = data[at + 8..at + 40].try_into().unwrap();
        (slot, hash)
    };
    let (newest, _) = entry(0);
    let (oldest, _) = entry(n - 1);
    if newest < target {
        return Ok(Entropy::TooEarly);
    }
    if oldest > target {
        return Ok(Entropy::Gone);
    }
    // Newest first, so walk back to the last entry still at or after target.
    let mut found = entry(0);
    for i in 1..n {
        let e = entry(i);
        if e.0 < target {
            break;
        }
        found = e;
    }
    Ok(Entropy::Found(found.0, found.1))
}

/// A guarantee its owner has asked for back. Still in the cover, and still
/// paying claims, until the notice runs out.
#[account]
#[derive(InitSpace)]
pub struct Exit {
    pub owner: Pubkey,
    pub shares: u64,
    pub ready_at: i64,
    pub bump: u8,
}

#[event]
pub struct Opened {
    pub usd_mint: Pubkey,
    pub coin_mint: Pubkey,
    pub depositor: Pubkey,
    pub amount: u64,
    pub coins_per_usd: u64,
}

#[event]
pub struct Deposited {
    pub depositor: Pubkey,
    pub amount: u64,
    pub into_pool: u64,
    pub free_coins: u64,
    pub guarantee_coins: u64,
}

#[event]
pub struct FeePaid {
    pub payer: Pubkey,
    pub markup: u64,
    pub reserve: u64,
    pub burned: u64,
    pub shared: u64,
}

#[event]
pub struct ReadingCommitted {
    pub reading: Pubkey,
    pub round: Pubkey,
    pub question_hash: [u8; 32],
    pub commit_slot: u64,
}

#[event]
pub struct ReadingRevealed {
    pub reading: Pubkey,
    pub reply_hash: [u8; 32],
    pub verdict: u8,
    pub reveal_slot: u64,
}

#[event]
pub struct QuoteSet {
    pub reading: Pubkey,
    pub member: u32,
    pub rate: u16,
    pub set_time: i64,
}

#[event]
pub struct MemberJoined {
    pub owner: Pubkey,
    pub number: u32,
    pub seat: u32,
    pub stake: u64,
}

#[event]
pub struct MemberLeaving {
    pub number: u32,
    pub leave_at: i64,
}

#[event]
pub struct MemberLeft {
    pub number: u32,
    pub stake: u64,
}

#[event]
pub struct Challenged {
    pub reading: Pubkey,
    pub challenger: Pubkey,
    pub deadline: i64,
}

#[event]
pub struct ChallengeAnswered {
    pub reading: Pubkey,
}

#[event]
pub struct ChallengeUpheld {
    pub reading: Pubkey,
    pub number: u32,
    pub stake_taken: u64,
}

#[event]
pub struct PairSettled {
    pub pair: Pubkey,
    pub first: Pubkey,
    pub second: Pubkey,
    pub outcome: u8,
}

#[event]
pub struct ReadingLapsed {
    pub reading: Pubkey,
}

#[event]
pub struct RoundOpened {
    pub round: Pubkey,
    pub opener: Pubkey,
    pub pool_fingerprint: [u8; 32],
    pub pool_size: u32,
    pub count: u16,
    pub commit_slot: u64,
}

#[event]
pub struct RoundDrawn {
    pub round: Pubkey,
    pub entropy_slot: u64,
    pub final_seed: [u8; 32],
}

#[event]
pub struct RoundWithheld {
    pub round: Pubkey,
}

#[event]
pub struct JoinedCover {
    pub owner: Pubkey,
    pub coins: u64,
    pub shares: u64,
}

#[event]
pub struct Claimed {
    pub claimant: Pubkey,
    pub dollars: u64,
    pub coins_burned: u64,
    pub reference: [u8; 32],
}

#[event]
pub struct ReleaseRequested {
    pub owner: Pubkey,
    pub shares: u64,
    pub ready_at: i64,
}

#[event]
pub struct Released {
    pub owner: Pubkey,
    pub shares: u64,
    pub coins: u64,
}

#[event]
pub struct DepthAdded {
    pub giver: Pubkey,
    pub amount: u64,
}

#[event]
pub struct EntryFeesSettled {
    pub amount: u64,
    pub burned: u64,
    pub shared: u64,
}

// ---------------------------------------------------------------- arithmetic

/// The markup on a purchase, cut into its three uses, in dollars. The
/// participants' part is what is left, so the three always add back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fee {
    pub reserve: u64,
    pub burn: u64,
    pub participants: u64,
}

impl Fee {
    pub fn of(markup: u64) -> Result<Self> {
        let reserve = u64::try_from(markup as u128 * RESERVE_POINTS as u128 / MARKUP_POINTS as u128)
            .map_err(|_| SasonaError::Overflow)?;
        let burn = burn_of(markup)?.min(markup - reserve);
        let participants = markup
            .checked_sub(reserve)
            .and_then(|x| x.checked_sub(burn))
            .ok_or(SasonaError::Overflow)?;
        Ok(Self { reserve, burn, participants })
    }

    /// The dollars that buy coin: the burn's and the participants'.
    pub fn buy(&self) -> u64 {
        self.burn + self.participants
    }
}

/// Shares bought by `coins` added to a cover holding `cover_coins` behind
/// `cover_shares`, rounded down. The first coins in buy shares one for one.
pub fn shares_for(coins: u64, cover_shares: u64, cover_coins: u64) -> Result<u64> {
    if cover_shares == 0 {
        return Ok(coins);
    }
    require!(cover_coins > 0, SasonaError::PoolEmpty);
    let v = coins as u128 * cover_shares as u128 / cover_coins as u128;
    u64::try_from(v).map_err(|_| SasonaError::Overflow.into())
}

/// Coins behind `shares` of a cover, rounded down.
pub fn coins_for_shares(shares: u64, cover_shares: u64, cover_coins: u64) -> Result<u64> {
    require!(cover_shares > 0 && shares <= cover_shares, SasonaError::MoreThanIsThere);
    let v = shares as u128 * cover_coins as u128 / cover_shares as u128;
    u64::try_from(v).map_err(|_| SasonaError::Overflow.into())
}

/// The burn's share of an amount, rounded up so that it is never less than
/// three percent, however small the payment.
pub fn burn_of(amount: u64) -> Result<u64> {
    let v = (amount as u128 * BURN_BPS as u128).div_ceil(10_000);
    u64::try_from(v).map_err(|_| SasonaError::Overflow.into())
}

/// Coins out of the pool for `dollars` in, at constant product, rounded down.
pub fn coins_out(dollars: u64, usd: u64, coins: u64) -> Result<u64> {
    require!(usd > 0, SasonaError::PoolEmpty);
    let v = coins as u128 * dollars as u128 / (usd as u128 + dollars as u128);
    u64::try_from(v).map_err(|_| SasonaError::Overflow.into())
}

/// A deposit cut into its four parts. `free` is what is left after the other
/// two, so the parts always add back to the deposit exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slices {
    pub fee: u64,
    pub rest: u64,
    pub spread: u64,
    pub guarantee: u64,
    pub free: u64,
}

impl Slices {
    pub fn of(amount: u64) -> Result<Self> {
        let fee = bps(amount, FEE_BPS)?;
        let rest = amount.checked_sub(fee).ok_or(SasonaError::Overflow)?;
        let spread = bps(rest, SPREAD_BPS)?;
        let guarantee = bps(rest, GUARANTEE_BPS)?;
        let free = rest
            .checked_sub(spread)
            .and_then(|x| x.checked_sub(guarantee))
            .ok_or(SasonaError::Overflow)?;
        Ok(Self { fee, rest, spread, guarantee, free })
    }
}

pub fn bps(amount: u64, bps: u64) -> Result<u64> {
    let v = (amount as u128) * (bps as u128) / 10_000;
    u64::try_from(v).map_err(|_| SasonaError::Overflow.into())
}

/// Dollars converted to coins at the ratio coins/usd, rounded down.
pub fn at_price(dollars: u64, coins: u64, usd: u64) -> Result<u64> {
    require!(usd > 0, SasonaError::PoolEmpty);
    let v = dollars as u128 * coins as u128 / usd as u128;
    u64::try_from(v).map_err(|_| SasonaError::Overflow.into())
}

fn times(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or(SasonaError::Overflow.into())
}

fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(SasonaError::Overflow.into())
}

// -------------------------------------------------------------------- errors

#[error_code]
pub enum SasonaError {
    #[msg("A deposit of nothing is not a deposit")]
    NothingDeposited,
    #[msg("That is not the token this pool counts as a dollar")]
    NotTheDollar,
    #[msg("Coin supply does not equal what is in the pool plus what is outside it")]
    SupplyMismatch,
    #[msg("The pool holds fewer dollars than it has recorded")]
    DollarsMissing,
    #[msg("Too small to give any coins at the current price")]
    TooSmall,
    #[msg("A deposit moved the price")]
    PriceMoved,
    #[msg("The pool holds no dollars to price against")]
    PoolEmpty,
    #[msg("This guarantee is still in its old vault; it has to join the cover first")]
    JoinCoverFirst,
    #[msg("Only the judge can approve a claim, for now")]
    NotTheJudge,
    #[msg("More than there is")]
    MoreThanIsThere,
    #[msg("The notice has not run out yet")]
    NoticeNotOver,
    #[msg("That claim would leave the cover too thin for new deposits")]
    CoverTooThin,
    #[msg("A claim cannot be paid into the pool's own accounts")]
    NotTheClaimant,
    #[msg("A round needs at least one candidate, and no more picks than candidates")]
    BadRound,
    #[msg("This round is not waiting for its seed")]
    RoundNotOpen,
    #[msg("That seed does not match the round's commitment")]
    WrongSeed,
    #[msg("The slot whose hash the round needs does not exist yet")]
    TooEarly,
    #[msg("The slot whose hash the round needs is no longer in SlotHashes; the round can only be marked withheld")]
    TooLate,
    #[msg("The round can still be revealed")]
    NotWithheldYet,
    #[msg("The SlotHashes sysvar could not be read")]
    BadSlotHashes,
    #[msg("Readings are only taken of a round that has been drawn")]
    RoundNotDrawn,
    #[msg("A service is a printable ASCII URL of at most 256 bytes, and its hash must match")]
    BadEndpoint,
    #[msg("Only the reader may do that")]
    NotTheReader,
    #[msg("This reading is not waiting for its question")]
    ReadingNotOpen,
    #[msg("The question for that nonce is not the one committed")]
    WrongQuestion,
    #[msg("A verdict is 1 delivered, 2 wrong answer or 3 empty")]
    BadVerdict,
    #[msg("The reading can still be revealed")]
    NotLapsedYet,
    #[msg("That nonce belongs to a reading committed before this one")]
    NonceTaken,
    #[msg("A second reading re-tests a reading that has been revealed")]
    FirstNotRevealed,
    #[msg("A second reading is of the same service as the first")]
    NotTheSameService,
    #[msg("A second reading is taken in another round")]
    SameRound,
    #[msg("A second reading is taken by someone other than the first reader")]
    SameReader,
    #[msg("This pair has already been settled")]
    AlreadySettled,
    #[msg("That membership was not drawn to read this service")]
    NotDrawn,
    #[msg("A seat passed over in the draw holds a membership that could have read")]
    NotSkippable,
    #[msg("This membership is not active")]
    NotActive,
    #[msg("This membership has not asked to leave")]
    NotLeaving,
    #[msg("Only the membership's owner can do this")]
    NotTheOwner,
    #[msg("A challenge to one of this membership's readings is still open")]
    ChallengeOpen,
    #[msg("This reading was taken before members, and has no stake behind it")]
    NoMember,
    #[msg("This challenge has already been settled")]
    ChallengeClosed,
    #[msg("A reply is at most 10,000 bytes")]
    ReplyTooLong,
    #[msg("The reply on chain does not hash to what the reading recorded")]
    NotTheReply,
    #[msg("The verdict rule on that reply does not give the recorded verdict")]
    VerdictDoesNotFollow,
    #[msg("That is not who challenged")]
    NotTheChallenger,
    #[msg("That is not the seat named, or not the membership in it")]
    NotTheSeat,
    #[msg("The reply on chain was answered with and can no longer change")]
    EvidenceSealed,
    #[msg("The time allowed for this has passed")]
    WindowClosed,
    #[msg("This membership sat down in its seat after the round was committed")]
    SatDownSince,
    #[msg("A quote is between 1 and 10,000 basis points, or 0 to withdraw it")]
    BadRate,
    #[msg("Only a reading that says delivered can be insured")]
    NotDelivered,
    #[msg("Arithmetic overflow")]
    Overflow,
}
