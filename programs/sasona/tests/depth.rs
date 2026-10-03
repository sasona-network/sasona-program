//! Adding depth, run against the compiled program.

mod common;
use common::*;

fn opened() -> World {
    let mut w = world();
    open(&mut w, 1_220 * DOLLAR);
    w
}

#[test]
fn depth_adds_dollars_and_mints_nothing() {
    let mut w = opened();
    let (k, from) = newcomer(&mut w.svm, 100);
    let before = pool(&w.svm);
    let supply = mint_state(&w.svm, pda(&[COIN_SEED])).supply;

    try_add_depth(&mut w.svm, &k, from, 50 * DOLLAR).unwrap();

    let after = pool(&w.svm);
    assert_eq!(after.usd_reserve, before.usd_reserve + 50 * DOLLAR);
    assert_eq!(after.coin_reserve, before.coin_reserve);
    assert_eq!(after.outside, before.outside);
    assert_eq!(mint_state(&w.svm, pda(&[COIN_SEED])).supply, supply, "nothing minted");
    assert_eq!(token_balance(&w.svm, from), 50 * DOLLAR);
    // Same coins, more dollars: the price went up.
    assert!(after.usd_reserve as u128 * before.coin_reserve as u128 > before.usd_reserve as u128 * after.coin_reserve as u128);
    assert_books_balance(&w.svm);
}

#[test]
fn deposits_after_depth_are_priced_at_the_new_ratio() {
    let mut w = opened();
    let (k, from) = newcomer(&mut w.svm, 1_000);
    try_add_depth(&mut w.svm, &k, from, 333 * DOLLAR + 7).unwrap();
    let before = pool(&w.svm);
    try_deposit(&mut w.svm, &k, from, 100 * DOLLAR).unwrap();
    let after = pool(&w.svm);
    let lhs = before.coin_reserve as u128 * after.usd_reserve as u128;
    let rhs = after.coin_reserve as u128 * before.usd_reserve as u128;
    assert!(lhs >= rhs && lhs - rhs < before.usd_reserve as u128);
    let s = Slices::of(100 * DOLLAR).unwrap();
    assert_eq!(token_balance(&w.svm, ata(k.pubkey(), pda(&[COIN_SEED]))),
               at_price(s.free, before.coin_reserve, before.usd_reserve).unwrap());
    assert_books_balance(&w.svm);
}

#[test]
fn nothing_added_is_refused() {
    let mut w = opened();
    let (k, from) = newcomer(&mut w.svm, 100);
    let err = try_add_depth(&mut w.svm, &k, from, 0).unwrap_err();
    assert!(err.contains("NothingDeposited"), "{err}");
}

#[test]
fn depth_in_a_dollar_of_their_own_is_refused() {
    let mut w = opened();
    let (k, _) = newcomer(&mut w.svm, 0);
    let fake = Address::new_unique();
    put_mint(&mut w.svm, fake, k.pubkey(), 6);
    let fake_usd = Address::new_unique();
    put_token_account(&mut w.svm, fake_usd, fake, k.pubkey(), 100 * DOLLAR);
    w.svm.expire_blockhash();
    let err = send(&mut w.svm, add_depth_ix(k.pubkey(), fake, fake_usd, DOLLAR), &[&k]).unwrap_err();
    assert!(err.contains("NotTheDollar"), "{err}");
    let err = try_add_depth(&mut w.svm, &k, fake_usd, DOLLAR).unwrap_err();
    assert!(err.contains("ConstraintTokenMint"), "{err}");
}

#[test]
fn depth_from_someone_elses_dollars_is_refused() {
    let mut w = opened();
    let (k, _) = newcomer(&mut w.svm, 0);
    let (_, theirs) = newcomer(&mut w.svm, 100);
    let err = try_add_depth(&mut w.svm, &k, theirs, DOLLAR).unwrap_err();
    assert!(err.contains("ConstraintTokenOwner"), "{err}");
}

#[test]
fn a_look_alike_pool_dollar_account_is_refused() {
    let mut w = opened();
    let (k, from) = newcomer(&mut w.svm, 100);
    let look_alike = Address::new_unique();
    put_token_account(&mut w.svm, look_alike, usd(), k.pubkey(), u64::MAX / 4);
    let mut ix = add_depth_ix(k.pubkey(), usd(), from, DOLLAR);
    ix.accounts[4].pubkey = look_alike;
    w.svm.expire_blockhash();
    let err = send(&mut w.svm, ix, &[&k]).unwrap_err();
    assert!(err.contains("ConstraintSeeds"), "{err}");
}
