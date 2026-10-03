//! Opening the pool, run against the compiled program in a local simulator.
//!
//! Build first: `bash scripts/build.sh` builds the binary and then runs these
//! against it. Several tests are attacks, and they pass when the attack is
//! refused for the reason we expect.


mod common;
use common::*;

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
