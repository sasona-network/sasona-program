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
use anchor_spl::token::{self, Burn, Mint, MintTo, Token, TokenAccount, Transfer};

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
        token::mint_to(a.cpi_mint(&a.guarantee_vault).with_signer(signer), guarantee_coins)?;

        let pool = &mut ctx.accounts.pool;
        pool.usd_reserve = s.rest;
        pool.coin_reserve = into_pool;
        pool.outside = add(free_coins, guarantee_coins)?;
        pool.fees_held = s.fee;

        let g = &mut ctx.accounts.guarantee;
        g.owner = ctx.accounts.depositor.key();
        g.coins = guarantee_coins;
        g.bump = ctx.bumps.guarantee;

        ctx.accounts.coin_mint.reload()?;
        ctx.accounts.pool_usd.reload()?;
        ctx.accounts.fees.reload()?;
        let a = &ctx.accounts;
        a.pool.check(a.coin_mint.supply, a.pool_usd.amount, a.fees.amount)?;

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
        token::mint_to(a.cpi_mint(&a.guarantee_vault).with_signer(signer), guarantee_coins)?;

        let pool = &mut ctx.accounts.pool;
        pool.usd_reserve = add(usd_before, s.rest)?;
        pool.coin_reserve = add(coin_before, into_pool)?;
        pool.outside = add(pool.outside, add(free_coins, guarantee_coins)?)?;
        pool.fees_held = add(pool.fees_held, s.fee)?;
        pool.price_held(usd_before, coin_before)?;

        // A first deposit creates the guarantee; a later one adds to it.
        let g = &mut ctx.accounts.guarantee;
        if g.owner == Pubkey::default() {
            g.owner = ctx.accounts.depositor.key();
            g.bump = ctx.bumps.guarantee;
        }
        g.coins = add(g.coins, guarantee_coins)?;

        ctx.accounts.coin_mint.reload()?;
        ctx.accounts.pool_usd.reload()?;
        ctx.accounts.fees.reload()?;
        let a = &ctx.accounts;
        a.pool.check(a.coin_mint.supply, a.pool_usd.amount, a.fees.amount)?;

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

    /// Held by the pool, not the depositor, so the depositor cannot move it.
    #[account(init, payer = depositor, seeds = [VAULT_SEED, depositor.key().as_ref()], bump,
              token::mint = coin_mint, token::authority = pool)]
    pub guarantee_vault: Account<'info, TokenAccount>,

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

    /// These three already exist if this depositor has deposited before, and
    /// then the constraints are checked against what is there.
    #[account(init_if_needed, payer = depositor,
              associated_token::mint = coin_mint, associated_token::authority = depositor)]
    pub depositor_coin: Account<'info, TokenAccount>,

    #[account(init_if_needed, payer = depositor, seeds = [VAULT_SEED, depositor.key().as_ref()], bump,
              token::mint = coin_mint, token::authority = pool)]
    pub guarantee_vault: Account<'info, TokenAccount>,

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
    pub coins: u64,
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
    #[msg("Arithmetic overflow")]
    Overflow,
}
