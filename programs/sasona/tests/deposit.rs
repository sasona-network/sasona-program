//! Deposits into an open pool, run against the compiled program.

mod common;
use common::*;

fn opened() -> World {
    let mut w = world();
    open(&mut w, 1_220 * DOLLAR);
    w
}

// ----------------------------------------------------------------- it works

#[test]
fn a_newcomer_deposits_at_the_pool_price() {
    let mut w = opened();
    let before = pool(&w.svm);
    let (k, from) = newcomer(&mut w.svm, 1_000);
    try_deposit(&mut w.svm, &k, from, 100 * DOLLAR).unwrap();

    let s = Slices::of(100 * DOLLAR).unwrap();
    let after = pool(&w.svm);
    let coin = pda(&[COIN_SEED]);

    assert_eq!(after.usd_reserve, before.usd_reserve + s.rest);
    assert_eq!(after.fees_held, before.fees_held + s.fee);
    assert_eq!(token_balance(&w.svm, ata(k.pubkey(), coin)), s.free * PRICE);
    assert_eq!(shares_of(&w.svm, k.pubkey()), s.guarantee * PRICE, "no claims yet, so a share is still one coin");
    assert_eq!(token_balance(&w.svm, from), 900 * DOLLAR);

    // The ratio is exactly where it was.
    assert_eq!(after.coin_reserve as u128 * before.usd_reserve as u128,
               before.coin_reserve as u128 * after.usd_reserve as u128);

    let g: Guarantee = read(&w.svm, pda(&[GUARANTEE_SEED, k.pubkey().as_ref()]));
    assert_eq!(addr(g.owner), k.pubkey());
    assert_eq!(g.shares, s.guarantee * PRICE);
    assert_books_balance(&w.svm);
}

#[test]
fn depositing_again_adds_to_the_same_guarantee() {
    let mut w = opened();
    let (k, from) = newcomer(&mut w.svm, 1_000);
    for amount in [100 * DOLLAR, 37 * DOLLAR + 1, 250 * DOLLAR] {
        try_deposit(&mut w.svm, &k, from, amount).unwrap();
    }
    let expected: u64 = [100 * DOLLAR, 37 * DOLLAR + 1, 250 * DOLLAR]
        .iter()
        .map(|a| Slices::of(*a).unwrap().guarantee * PRICE)
        .sum();
    assert_eq!(shares_of(&w.svm, k.pubkey()), expected);
    let c = cover(&w.svm);
    assert_eq!(c.coins, token_balance(&w.svm, pda(&[COVER_VAULT_SEED])));
    assert_books_balance(&w.svm);
}

#[test]
fn the_opener_can_deposit_again() {
    let mut w = opened();
    let d = w.depositor.insecure_clone();
    let from = w.depositor_usd;
    let before: Guarantee = read(&w.svm, pda(&[GUARANTEE_SEED, d.pubkey().as_ref()]));
    try_deposit(&mut w.svm, &d, from, 100 * DOLLAR).unwrap();
    let after: Guarantee = read(&w.svm, pda(&[GUARANTEE_SEED, d.pubkey().as_ref()]));
    assert_eq!(after.shares, before.shares + Slices::of(100 * DOLLAR).unwrap().guarantee * PRICE);
    assert_books_balance(&w.svm);
}

#[test]
fn many_deposits_of_odd_sizes_keep_the_books() {
    let mut w = opened();
    let people: Vec<(Keypair, Address)> = (0..4).map(|_| newcomer(&mut w.svm, 100_000)).collect();
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    for i in 0..40 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let amount = 20 + x % (5_000 * DOLLAR);
        let (k, from) = &people[i % people.len()];
        let before = pool(&w.svm);
        try_deposit(&mut w.svm, k, *from, amount).unwrap();
        let after = pool(&w.svm);
        let lhs = before.coin_reserve as u128 * after.usd_reserve as u128;
        let rhs = after.coin_reserve as u128 * before.usd_reserve as u128;
        assert!(lhs >= rhs && lhs - rhs < before.usd_reserve as u128, "price moved at deposit {i}");
        assert_books_balance(&w.svm);
    }
}

#[test]
fn a_gift_to_the_pool_does_not_block_deposits() {
    // Anyone can send tokens to the pool's accounts. That must not stop the
    // pool from working.
    let mut w = opened();
    for at in [pda(&[POOL_USD_SEED]), pda(&[FEES_SEED])] {
        let bal = token_balance(&w.svm, at);
        put_token_account(&mut w.svm, at, usd(), pda(&[POOL_SEED]), bal + 1);
    }
    let (k, from) = newcomer(&mut w.svm, 1_000);
    try_deposit(&mut w.svm, &k, from, 100 * DOLLAR).unwrap();
    assert_books_balance(&w.svm);
}

// ------------------------------------------------- the arithmetic, directly

#[test]
fn rounding_never_lets_the_price_fall() {
    // On chain the ratio is still a whole number, so nothing rounds yet. Here
    // it is not, to check the rule that will matter once fees move the price.
    let mut x: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut checked = 0;
    for _ in 0..100_000 {
        let usd = 1 + next() % (1u64 << 40);
        let coins = 1 + next() % (1u64 << 52);
        let rest = 1 + next() % (1u64 << 40);
        // Too large to fit is refused by the program; that is a separate rule.
        let Ok(minted) = at_price(rest, coins, usd) else { continue };
        let Some(coin_reserve) = coins.checked_add(minted) else { continue };
        checked += 1;
        let after = Pool {
            bump: 0,
            usd_mint: Default::default(),
            coin_mint: Default::default(),
            usd_reserve: usd + rest,
            coin_reserve,
            outside: 0,
            fees_held: 0,
        };
        assert!(after.price_held(usd, coins).is_ok(), "usd {usd} coins {coins} rest {rest}");
    }
    assert!(checked > 50_000, "only {checked} cases were checked");
}

#[test]
fn rounding_up_would_be_caught() {
    let after = Pool {
        bump: 0,
        usd_mint: Default::default(),
        coin_mint: Default::default(),
        usd_reserve: 3 + 1,
        coin_reserve: 10 + 4, // 1 dollar at 10/3 is 3.33 coins; 4 rounds up
        outside: 0,
        fees_held: 0,
    };
    assert!(after.price_held(3, 10).is_err());
}

// ------------------------------------------------------------- refusals

#[test]
fn nothing_can_be_deposited_before_the_pool_opens() {
    let mut w = world();
    let (k, from) = newcomer(&mut w.svm, 1_000);
    let err = try_deposit(&mut w.svm, &k, from, 100 * DOLLAR).unwrap_err();
    assert!(err.contains("AccountNotInitialized"), "{err}");
}

#[test]
fn nothing_deposited_is_refused() {
    let mut w = opened();
    let (k, from) = newcomer(&mut w.svm, 1_000);
    let err = try_deposit(&mut w.svm, &k, from, 0).unwrap_err();
    assert!(err.contains("NothingDeposited"), "{err}");
}

#[test]
fn a_deposit_too_small_to_give_coins_is_refused() {
    let mut w = opened();
    let (k, from) = newcomer(&mut w.svm, 1_000);
    let err = try_deposit(&mut w.svm, &k, from, 1).unwrap_err();
    assert!(err.contains("TooSmall"), "{err}");
    assert_eq!(token_balance(&w.svm, from), 1_000 * DOLLAR);
}

#[test]
fn a_dollar_of_their_own_is_refused() {
    let mut w = opened();
    let (k, _) = newcomer(&mut w.svm, 0);
    let fake = Address::new_unique();
    put_mint(&mut w.svm, fake, k.pubkey(), 6);
    let fake_usd = Address::new_unique();
    put_token_account(&mut w.svm, fake_usd, fake, k.pubkey(), 1_000 * DOLLAR);

    w.svm.expire_blockhash();
    let err = send(&mut w.svm, deposit_ix(k.pubkey(), fake, fake_usd, 100 * DOLLAR), &[&k]).unwrap_err();
    assert!(err.contains("NotTheDollar"), "{err}");

    let err = try_deposit(&mut w.svm, &k, fake_usd, 100 * DOLLAR).unwrap_err();
    assert!(err.contains("ConstraintTokenMint"), "{err}");
}

#[test]
fn someone_elses_dollars_are_refused() {
    let mut w = opened();
    let (k, _) = newcomer(&mut w.svm, 0);
    let (_, theirs) = newcomer(&mut w.svm, 1_000);
    let err = try_deposit(&mut w.svm, &k, theirs, 100 * DOLLAR).unwrap_err();
    assert!(err.contains("ConstraintTokenOwner"), "{err}");
    assert_eq!(token_balance(&w.svm, theirs), 1_000 * DOLLAR);
}

#[test]
fn nobody_can_deposit_into_someone_elses_guarantee() {
    let mut w = opened();
    let (k, from) = newcomer(&mut w.svm, 1_000);
    let opener = w.depositor.pubkey();
    let mut ix = deposit_ix(k.pubkey(), usd(), from, 100 * DOLLAR);
    // Swap in the opener's guarantee record.
    ix.accounts[12].pubkey = pda(&[GUARANTEE_SEED, opener.as_ref()]);
    w.svm.expire_blockhash();
    let err = send(&mut w.svm, ix, &[&k]).unwrap_err();
    assert!(err.contains("ConstraintSeeds"), "{err}");
}

/// Pass an account of the depositor's own where one of the pool's belongs.
/// Each is given a balance large enough that the balance checks alone would
/// not notice, so only the address check can refuse it.
fn swap_in_own_account(index: usize, mint: Address) {
    let mut w = opened();
    let (k, from) = newcomer(&mut w.svm, 1_000);
    let look_alike = Address::new_unique();
    put_token_account(&mut w.svm, look_alike, mint, k.pubkey(), u64::MAX / 4);
    let mut ix = deposit_ix(k.pubkey(), usd(), from, 100 * DOLLAR);
    ix.accounts[index].pubkey = look_alike;
    w.svm.expire_blockhash();
    let err = send(&mut w.svm, ix, &[&k]).unwrap_err();
    assert!(err.contains("ConstraintSeeds"), "{err}");
}

#[test]
fn a_look_alike_pool_dollar_account_is_refused() {
    swap_in_own_account(4, usd());
}

#[test]
fn a_look_alike_pool_coin_account_is_refused() {
    swap_in_own_account(5, pda(&[COIN_SEED]));
}

#[test]
fn a_look_alike_fee_account_is_refused() {
    swap_in_own_account(6, usd());
}

#[test]
fn free_coins_only_go_to_the_depositors_own_account() {
    let mut w = opened();
    let (k, from) = newcomer(&mut w.svm, 1_000);
    let elsewhere = Address::new_unique();
    put_token_account(&mut w.svm, elsewhere, pda(&[COIN_SEED]), k.pubkey(), 0);
    let mut ix = deposit_ix(k.pubkey(), usd(), from, 100 * DOLLAR);
    ix.accounts[8].pubkey = elsewhere;
    w.svm.expire_blockhash();
    let err = send(&mut w.svm, ix, &[&k]).unwrap_err();
    assert!(err.contains("AccountNotAssociatedTokenAccount") || err.contains("ConstraintAssociated"), "{err}");
}

#[test]
fn someone_else_creating_the_coin_account_first_does_not_block_a_deposit() {
    let mut w = opened();
    let (k, from) = newcomer(&mut w.svm, 1_000);
    let (stranger, _) = newcomer(&mut w.svm, 0);
    let coin = pda(&[COIN_SEED]);
    let create = spl_associated_token_account_interface::instruction::create_associated_token_account(
        &stranger.pubkey(),
        &k.pubkey(),
        &coin,
        &token_program(),
    );
    w.svm.expire_blockhash();
    send(&mut w.svm, create, &[&stranger]).unwrap();
    try_deposit(&mut w.svm, &k, from, 100 * DOLLAR).unwrap();
    assert_eq!(token_balance(&w.svm, ata(k.pubkey(), coin)), Slices::of(100 * DOLLAR).unwrap().free * PRICE);
}

#[test]
fn a_deposit_too_small_to_pay_a_fee_is_refused() {
    let mut w = opened();
    let (k, from) = newcomer(&mut w.svm, 1_000);
    // 6 units: no fee and no spread, but some guarantee and free coins.
    let err = try_deposit(&mut w.svm, &k, from, 6).unwrap_err();
    assert!(err.contains("TooSmall"), "{err}");
}

#[test]
fn the_guarantee_still_cannot_be_taken_back() {
    let mut w = opened();
    let (k, from) = newcomer(&mut w.svm, 1_000);
    try_deposit(&mut w.svm, &k, from, 100 * DOLLAR).unwrap();
    let coin = pda(&[COIN_SEED]);
    let vault = pda(&[COVER_VAULT_SEED]);
    let ix = spl_token_interface::instruction::transfer(&token_program(), &vault, &ata(k.pubkey(), coin), &k.pubkey(), &[], 1)
        .unwrap();
    w.svm.expire_blockhash();
    let err = send(&mut w.svm, ix, &[&k]).unwrap_err();
    assert!(err.contains("owner does not match"), "{err}");
}
