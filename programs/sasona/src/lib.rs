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
use anchor_spl::token::{self, Mint, MintTo, Token, TokenAccount, Transfer};

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

// ---------------------------------------------------------------- arithmetic

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
    #[msg("Arithmetic overflow")]
    Overflow,
}
