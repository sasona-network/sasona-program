//! Opening the pool, run against the compiled program in a local simulator.
//!
//! Build first: `bash scripts/build.sh` builds the binary and then runs these
//! against it. Several tests are attacks, and they pass when the attack is
//! refused for the reason we expect.

use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use litesvm::LiteSVM;
use solana_account::Account;
use solana_address::Address;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_program_pack::Pack;
use solana_signer::Signer;
use solana_transaction::Transaction;
use spl_token_interface::state::{Account as TokenAccount, AccountState, Mint};

use sasona::{
    Guarantee, Pool, Slices, COIN_DECIMALS, COIN_SEED, FEES_SEED, GUARANTEE_SEED, OPENING_COINS_PER_USD,
    POOL_COIN_SEED, POOL_SEED, POOL_USD_SEED, USD_MINT, VAULT_SEED,
};

const DOLLAR: u64 = 1_000_000;
const PRICE: u64 = OPENING_COINS_PER_USD;

fn addr(p: anchor_lang::prelude::Pubkey) -> Address {
    Address::new_from_array(p.to_bytes())
}

fn key(a: Address) -> anchor_lang::prelude::Pubkey {
    anchor_lang::prelude::Pubkey::new_from_array(a.to_bytes())
}

fn program_id() -> Address {
    addr(sasona::ID)
}

fn pda(seeds: &[&[u8]]) -> Address {
    addr(anchor_lang::prelude::Pubkey::find_program_address(seeds, &sasona::ID).0)
}

fn token_program() -> Address {
    addr(anchor_spl::token::ID)
}

fn usd() -> Address {
    addr(USD_MINT)
}

fn ata(owner: Address, mint: Address) -> Address {
    addr(
        anchor_lang::prelude::Pubkey::find_program_address(
            &[owner.as_ref(), anchor_spl::token::ID.as_ref(), mint.as_ref()],
            &anchor_spl::associated_token::ID,
        )
        .0,
    )
}

struct World {
    svm: LiteSVM,
    depositor: Keypair,
    depositor_usd: Address,
}

fn put_mint(svm: &mut LiteSVM, at: Address, authority: Address, decimals: u8) {
    let mut data = vec![0u8; Mint::LEN];
    Mint {
        mint_authority: Some(authority).into(),
        supply: 0,
        decimals,
        is_initialized: true,
        freeze_authority: None.into(),
    }
    .pack_into_slice(&mut data);
    svm.set_account(at, Account { lamports: 1_000_000_000, data, owner: token_program(), executable: false, rent_epoch: 0 })
        .unwrap();
}

fn put_token_account(svm: &mut LiteSVM, at: Address, mint: Address, owner: Address, amount: u64) {
    let mut data = vec![0u8; TokenAccount::LEN];
    TokenAccount {
        mint,
        owner,
        amount,
        delegate: None.into(),
        state: AccountState::Initialized,
        is_native: None.into(),
        delegated_amount: 0,
        close_authority: None.into(),
    }
    .pack_into_slice(&mut data);
    svm.set_account(at, Account { lamports: 1_000_000_000, data, owner: token_program(), executable: false, rent_epoch: 0 })
        .unwrap();
}

fn world() -> World {
    let mut svm = LiteSVM::new();
    let so = std::env::var("SASONA_SO").expect("run through scripts/build.sh, which sets SASONA_SO");
    svm.add_program_from_file(program_id(), so).unwrap();

    let depositor = Keypair::new();
    svm.airdrop(&depositor.pubkey(), 100_000_000_000).unwrap();

    // The dollar lives at the address the program expects, as it does on devnet.
    put_mint(&mut svm, usd(), Address::new_unique(), 6);
    let depositor_usd = Address::new_unique();
    put_token_account(&mut svm, depositor_usd, usd(), depositor.pubkey(), 10_000 * DOLLAR);

    World { svm, depositor, depositor_usd }
}

fn open_ix(w: &World, usd_mint: Address, depositor_usd: Address, amount: u64) -> Instruction {
    let d = w.depositor.pubkey();
    let coin = pda(&[COIN_SEED]);
    let accounts = sasona::accounts::Open {
        depositor: key(d),
        pool: key(pda(&[POOL_SEED])),
        coin_mint: key(coin),
        usd_mint: key(usd_mint),
        pool_usd: key(pda(&[POOL_USD_SEED])),
        pool_coin: key(pda(&[POOL_COIN_SEED])),
        fees: key(pda(&[FEES_SEED])),
        depositor_usd: key(depositor_usd),
        depositor_coin: key(ata(d, coin)),
        guarantee_vault: key(pda(&[VAULT_SEED, d.as_ref()])),
        guarantee: key(pda(&[GUARANTEE_SEED, d.as_ref()])),
        token_program: anchor_spl::token::ID,
        associated_token_program: anchor_spl::associated_token::ID,
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None)
    .into_iter()
    .map(|m| AccountMeta { pubkey: addr(m.pubkey), is_signer: m.is_signer, is_writable: m.is_writable })
    .collect();
    let data = sasona::instruction::Open { amount }.data();
    Instruction { program_id: program_id(), accounts, data }
}

fn send(svm: &mut LiteSVM, ix: Instruction, signers: &[&Keypair]) -> Result<(), String> {
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&signers[0].pubkey()), signers, svm.latest_blockhash());
    svm.send_transaction(tx).map(|_| ()).map_err(|e| format!("{:?}\n{}", e.err, e.meta.logs.join("\n")))
}

fn token_balance(svm: &LiteSVM, at: Address) -> u64 {
    TokenAccount::unpack(&svm.get_account(&at).unwrap().data).unwrap().amount
}

fn mint_state(svm: &LiteSVM, at: Address) -> Mint {
    Mint::unpack(&svm.get_account(&at).unwrap().data).unwrap()
}

fn read<T: AccountDeserialize>(svm: &LiteSVM, at: Address) -> T {
    T::try_deserialize(&mut svm.get_account(&at).unwrap().data.as_slice()).unwrap()
}

fn try_open(w: &mut World, usd_mint: Address, depositor_usd: Address, amount: u64) -> Result<(), String> {
    let ix = open_ix(w, usd_mint, depositor_usd, amount);
    let d = w.depositor.insecure_clone();
    send(&mut w.svm, ix, &[&d])
}

fn open(w: &mut World, amount: u64) {
    let from = w.depositor_usd;
    try_open(w, usd(), from, amount).expect("open should succeed");
}

// ----------------------------------------------------------------- it works

#[test]
fn opening_splits_the_deposit_and_mints_against_it() {
    let mut w = world();
    let amount = 1_220 * DOLLAR;
    open(&mut w, amount);

    let s = Slices::of(amount).unwrap();
    let d = w.depositor.pubkey();
    let coin = pda(&[COIN_SEED]);

    assert_eq!(token_balance(&w.svm, pda(&[FEES_SEED])), s.fee);
    assert_eq!(token_balance(&w.svm, pda(&[POOL_USD_SEED])), s.rest);
    assert_eq!(token_balance(&w.svm, w.depositor_usd), 10_000 * DOLLAR - amount);

    let in_pool = token_balance(&w.svm, pda(&[POOL_COIN_SEED]));
    let free = token_balance(&w.svm, ata(d, coin));
    let locked = token_balance(&w.svm, pda(&[VAULT_SEED, d.as_ref()]));
    assert_eq!(in_pool, s.rest * PRICE);
    assert_eq!(free, s.free * PRICE);
    assert_eq!(locked, s.guarantee * PRICE);

    // Every coin that exists is in one of those three places.
    assert_eq!(mint_state(&w.svm, coin).supply, in_pool + free + locked);
}

#[test]
fn the_records_match_the_balances() {
    let mut w = world();
    let amount = 1_220 * DOLLAR;
    open(&mut w, amount);
    let s = Slices::of(amount).unwrap();
    let d = w.depositor.pubkey();

    let pool: Pool = read(&w.svm, pda(&[POOL_SEED]));
    assert_eq!(addr(pool.usd_mint), usd());
    assert_eq!(addr(pool.coin_mint), pda(&[COIN_SEED]));
    assert_eq!(pool.usd_reserve, s.rest);
    assert_eq!(pool.coin_reserve, s.rest * PRICE);
    assert_eq!(pool.outside, (s.free + s.guarantee) * PRICE);
    assert_eq!(pool.fees_held, s.fee);

    let g: Guarantee = read(&w.svm, pda(&[GUARANTEE_SEED, d.as_ref()]));
    assert_eq!(addr(g.owner), d);
    assert_eq!(g.coins, s.guarantee * PRICE);
}

#[test]
fn a_hundred_dollars_slices_as_designed() {
    let s = Slices::of(100 * DOLLAR).unwrap();
    assert_eq!(s.fee, 15 * DOLLAR);
    assert_eq!(s.spread, 12_750_000);
    assert_eq!(s.guarantee, 63_750_000);
    assert_eq!(s.free, 8_500_000);
}

#[test]
fn the_slices_always_add_back_to_the_deposit() {
    for amount in [1u64, 7, 333, 10_000, DOLLAR, 1_220 * DOLLAR, u64::MAX / 10_000] {
        let s = Slices::of(amount).unwrap();
        assert_eq!(s.fee + s.spread + s.guarantee + s.free, amount, "at {amount}");
        assert_eq!(s.fee + s.rest, amount, "at {amount}");
    }
}

// ----------------------------------------------------- nobody else can mint

#[test]
fn only_the_pool_can_mint_and_nobody_can_freeze() {
    let mut w = world();
    open(&mut w, 100 * DOLLAR);
    let m = mint_state(&w.svm, pda(&[COIN_SEED]));
    let pool = pda(&[POOL_SEED]);
    assert_eq!(Option::<Address>::from(m.mint_authority), Some(pool));
    assert_eq!(Option::<Address>::from(m.freeze_authority), None);
    assert_eq!(m.decimals, COIN_DECIMALS);
}

#[test]
fn the_pool_address_has_no_private_key() {
    // Derived from the program and off the ed25519 curve, so no key for it
    // can exist. This says nothing about the program's upgrade authority,
    // which the proof on devnet names separately.
    assert!(!pda(&[POOL_SEED]).is_on_curve());
}

#[test]
fn the_depositor_cannot_mint() {
    let mut w = world();
    open(&mut w, 100 * DOLLAR);
    let d = w.depositor.insecure_clone();
    let coin = pda(&[COIN_SEED]);
    let ix = spl_token_interface::instruction::mint_to(&token_program(), &coin, &ata(d.pubkey(), coin), &d.pubkey(), &[], 1)
        .unwrap();
    let err = send(&mut w.svm, ix, &[&d]).unwrap_err();
    assert!(err.contains("owner does not match"), "{err}");
}

// ---------------------------------------------------- the guarantee is held

#[test]
fn the_depositor_cannot_take_their_guarantee_back() {
    let mut w = world();
    open(&mut w, 100 * DOLLAR);
    let d = w.depositor.insecure_clone();
    let coin = pda(&[COIN_SEED]);
    let vault = pda(&[VAULT_SEED, d.pubkey().as_ref()]);
    let ix = spl_token_interface::instruction::transfer(&token_program(), &vault, &ata(d.pubkey(), coin), &d.pubkey(), &[], 1)
        .unwrap();
    let err = send(&mut w.svm, ix, &[&d]).unwrap_err();
    assert!(err.contains("owner does not match"), "{err}");
    assert_eq!(token_balance(&w.svm, vault), Slices::of(100 * DOLLAR).unwrap().guarantee * PRICE);
}

// ------------------------------------------------------- it opens only once

#[test]
fn a_second_opening_is_refused() {
    let mut w = world();
    open(&mut w, 100 * DOLLAR);
    w.svm.expire_blockhash();
    let from = w.depositor_usd;
    let err = try_open(&mut w, usd(), from, 100 * DOLLAR).unwrap_err();
    assert!(err.contains("already in use"), "{err}");
}

// ------------------------------------------------------ the dollar is fixed

#[test]
fn nobody_can_open_with_a_dollar_of_their_own() {
    // The attack the review found: open first, with a token you can mint
    // yourself, and the pool is backed by it for ever.
    let mut w = world();
    let fake = Address::new_unique();
    put_mint(&mut w.svm, fake, w.depositor.pubkey(), 6);
    let fake_usd = Address::new_unique();
    put_token_account(&mut w.svm, fake_usd, fake, w.depositor.pubkey(), 10_000 * DOLLAR);

    let err = try_open(&mut w, fake, fake_usd, 100 * DOLLAR).unwrap_err();
    assert!(err.contains("NotTheDollar"), "{err}");
}

#[test]
fn paying_in_a_different_token_is_refused() {
    let mut w = world();
    let fake = Address::new_unique();
    put_mint(&mut w.svm, fake, w.depositor.pubkey(), 6);
    let fake_usd = Address::new_unique();
    put_token_account(&mut w.svm, fake_usd, fake, w.depositor.pubkey(), 10_000 * DOLLAR);

    let err = try_open(&mut w, usd(), fake_usd, 100 * DOLLAR).unwrap_err();
    assert!(err.contains("ConstraintTokenMint"), "{err}");
}

#[test]
fn a_dollar_with_the_wrong_decimals_is_refused() {
    let mut w = world();
    put_mint(&mut w.svm, usd(), Address::new_unique(), 9);
    let from = w.depositor_usd;
    let err = try_open(&mut w, usd(), from, 100 * DOLLAR).unwrap_err();
    assert!(err.contains("ConstraintMintDecimals"), "{err}");
}

#[test]
fn someone_elses_dollars_are_refused() {
    let mut w = world();
    let theirs = Address::new_unique();
    put_token_account(&mut w.svm, theirs, usd(), Address::new_unique(), 10_000 * DOLLAR);

    let err = try_open(&mut w, usd(), theirs, 100 * DOLLAR).unwrap_err();
    assert!(err.contains("ConstraintTokenOwner"), "{err}");
    assert_eq!(token_balance(&w.svm, theirs), 10_000 * DOLLAR);
}

// --------------------------------------------------------- bad amounts

#[test]
fn nothing_deposited_is_refused() {
    let mut w = world();
    let from = w.depositor_usd;
    let err = try_open(&mut w, usd(), from, 0).unwrap_err();
    assert!(err.contains("NothingDeposited"), "{err}");
}

#[test]
fn an_amount_that_overflows_is_refused_not_wrapped() {
    let mut w = world();
    let from = w.depositor_usd;
    let err = try_open(&mut w, usd(), from, u64::MAX).unwrap_err();
    assert!(err.contains("Overflow"), "{err}");
}
