//! Fees and the entry fees deposits leave behind, run against the compiled program.

mod common;
use common::*;

fn opened() -> World {
    let mut w = world();
    open(&mut w, 1_220 * DOLLAR);
    w
}

fn network_coins(svm: &LiteSVM) -> u64 {
    svm.get_account(&pda(&[NETWORK_SEED]))
        .map(|_| token_balance(svm, pda(&[NETWORK_SEED])))
        .unwrap_or(0)
}

// ------------------------------------------------------------- the numbers

#[test]
fn a_markup_splits_as_the_design_says() {
    // A $10.00 purchase carries a $1.50 markup.
    let f = Fee::of(1_500_000).unwrap();
    assert_eq!(f.reserve, 500_000, "five of the fifteen points");
    assert_eq!(f.burn, 45_000, "0.45% of the $10.00 purchase");
    assert_eq!(f.participants, 955_000);
}

#[test]
fn the_burn_is_never_less_than_three_percent() {
    // A $0.001 purchase has a markup of 150 units; 3% is 4.5, so 5 burn.
    assert_eq!(Fee::of(150).unwrap().burn, 5);
    for markup in 1..10_000u64 {
        let f = Fee::of(markup).unwrap();
        assert!(f.burn as u128 * 10_000 >= markup as u128 * BURN_BPS as u128, "at {markup}");
        assert!(f.burn <= f.buy(), "at {markup}");
    }
}

#[test]
fn the_parts_of_a_markup_always_add_back() {
    for markup in [1u64, 2, 3, 29, 30, 34, 1_000, 1_500_000, 777_777_777, u64::MAX / 10_000] {
        let f = Fee::of(markup).unwrap();
        assert_eq!(f.reserve + f.burn + f.participants, markup, "at {markup}");
    }
}

#[test]
fn buying_never_shrinks_the_pool() {
    let mut x: u64 = 0x1234_5678_9abc_def1;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut checked = 0;
    for _ in 0..100_000 {
        let usd = 1 + next() % (1u64 << 44);
        let coins = 1 + next() % (1u64 << 56);
        let paid = 1 + next() % (1u64 << 44);
        let out = coins_out(paid, usd, coins).unwrap();
        assert!(out < coins, "a buy can never empty the pool");
        let before = usd as u128 * coins as u128;
        let after = (usd + paid) as u128 * (coins - out) as u128;
        assert!(after >= before, "usd {usd} coins {coins} paid {paid}");
        checked += 1;
    }
    assert_eq!(checked, 100_000);
}

// ----------------------------------------------------------------- it works

#[test]
fn a_fee_adds_depth_burns_and_pays_the_network() {
    let mut w = opened();
    let (k, from) = newcomer(&mut w.svm, 100);
    let before = pool(&w.svm);
    let supply_before = mint_state(&w.svm, pda(&[COIN_SEED])).supply;

    let markup = 1_500_000;
    try_pay_fee(&mut w.svm, &k, from, markup).unwrap();

    let f = Fee::of(markup).unwrap();
    // The reserve comes off the top: it deepens the pool before the buy.
    let out = coins_out(f.buy(), before.usd_reserve + f.reserve, before.coin_reserve).unwrap();
    let burned = (out as u128 * f.burn as u128).div_ceil(f.buy() as u128) as u64;
    let after = pool(&w.svm);

    assert_eq!(token_balance(&w.svm, from), 100 * DOLLAR - markup);
    assert_eq!(after.usd_reserve, before.usd_reserve + markup, "all of it stays in the pool");
    assert_eq!(after.coin_reserve, before.coin_reserve - out);
    assert_eq!(network_coins(&w.svm), out - burned, "every seat is empty, so all of it is the network's");
    assert_eq!(mint_state(&w.svm, pda(&[COIN_SEED])).supply, supply_before - burned);
    assert!(burned > 0);

    // The price went up: fewer coins per dollar than before.
    let now = after.coin_reserve as u128 * before.usd_reserve as u128;
    let then = before.coin_reserve as u128 * after.usd_reserve as u128;
    assert!(now < then);
    assert_books_balance(&w.svm);
}

#[test]
fn entry_fees_waiting_from_deposits_are_settled() {
    let mut w = opened();
    let (k, from) = newcomer(&mut w.svm, 1_000);
    try_deposit(&mut w.svm, &k, from, 100 * DOLLAR).unwrap();
    let before = pool(&w.svm);
    assert_eq!(before.fees_held, 183 * DOLLAR + 15 * DOLLAR);
    let supply_before = mint_state(&w.svm, pda(&[COIN_SEED])).supply;

    // Anyone can do it, including someone with nothing in the pool.
    let (stranger, _) = newcomer(&mut w.svm, 0);
    try_settle(&mut w.svm, &stranger).unwrap();

    let amount = before.fees_held;
    let out = coins_out(amount, before.usd_reserve, before.coin_reserve).unwrap();
    let burned = (out as u128 * burn_of(amount).unwrap() as u128).div_ceil(amount as u128) as u64;
    let after = pool(&w.svm);
    assert_eq!(after.fees_held, 0);
    assert_eq!(token_balance(&w.svm, pda(&[FEES_SEED])), 0);
    assert_eq!(after.usd_reserve, before.usd_reserve + amount);
    assert_eq!(network_coins(&w.svm), out - burned);
    assert_eq!(mint_state(&w.svm, pda(&[COIN_SEED])).supply, supply_before - burned);
    assert_books_balance(&w.svm);

    let err = try_settle(&mut w.svm, &stranger).unwrap_err();
    assert!(err.contains("NothingDeposited"), "{err}");
}

#[test]
fn deposits_and_fees_in_any_order_keep_the_books() {
    // Once a fee has moved the price, coins per dollar is no longer a whole
    // number, so deposits really do round from here on.
    let mut w = opened();
    let people: Vec<(Keypair, Address)> = (0..4).map(|_| newcomer(&mut w.svm, 100_000)).collect();
    let mut x: u64 = 0x0bad_c0de_dead_beef;
    let mut settled = 0;
    for i in 0..60 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let (k, from) = &people[i % people.len()];
        match x % 3 {
            0 => {
                let before = pool(&w.svm);
                try_deposit(&mut w.svm, k, *from, 20 + x % (500 * DOLLAR)).unwrap();
                let after = pool(&w.svm);
                let lhs = before.coin_reserve as u128 * after.usd_reserve as u128;
                let rhs = after.coin_reserve as u128 * before.usd_reserve as u128;
                assert!(lhs >= rhs && lhs - rhs < before.usd_reserve as u128, "deposit {i} moved the price");
            }
            1 => try_pay_fee(&mut w.svm, k, *from, 1 + x % (50 * DOLLAR)).unwrap(),
            _ => {
                // Refused only when there is nothing waiting.
                let waiting = pool(&w.svm).fees_held;
                match try_settle(&mut w.svm, k) {
                    Ok(()) => settled += 1,
                    Err(e) => assert!(waiting == 0 && e.contains("NothingDeposited"), "settle {i}: {e}"),
                }
            }
        }
        assert_books_balance(&w.svm);
    }
    assert!(settled >= 5, "only {settled} settles ran");
    let p = pool(&w.svm);
    assert_ne!(p.coin_reserve as u128, p.usd_reserve as u128 * PRICE as u128, "the price never moved");
}

#[test]
fn settling_moves_only_the_recorded_fees() {
    // Dollars sent straight to the fee account are not entry fees, and stay.
    let mut w = opened();
    let held = pool(&w.svm).fees_held;
    put_token_account(&mut w.svm, pda(&[FEES_SEED]), usd(), pda(&[POOL_SEED]), held + 7);
    let (k, _) = newcomer(&mut w.svm, 0);
    try_settle(&mut w.svm, &k).unwrap();
    assert_eq!(token_balance(&w.svm, pda(&[FEES_SEED])), 7);
    assert_books_balance(&w.svm);
}

// ------------------------------------------------------------- refusals

#[test]
fn nothing_paid_is_refused() {
    let mut w = opened();
    let (k, from) = newcomer(&mut w.svm, 100);
    let err = try_pay_fee(&mut w.svm, &k, from, 0).unwrap_err();
    assert!(err.contains("NothingDeposited"), "{err}");
}

#[test]
fn a_fee_in_a_dollar_of_their_own_is_refused() {
    let mut w = opened();
    let (k, _) = newcomer(&mut w.svm, 0);
    let fake = Address::new_unique();
    put_mint(&mut w.svm, fake, k.pubkey(), 6);
    let fake_usd = Address::new_unique();
    put_token_account(&mut w.svm, fake_usd, fake, k.pubkey(), 100 * DOLLAR);

    w.svm.expire_blockhash();
    let err = send(&mut w.svm, pay_fee_ix(k.pubkey(), fake, fake_usd, DOLLAR), &[&k]).unwrap_err();
    assert!(err.contains("NotTheDollar"), "{err}");
    let err = try_pay_fee(&mut w.svm, &k, fake_usd, DOLLAR).unwrap_err();
    assert!(err.contains("ConstraintTokenMint"), "{err}");
}

#[test]
fn a_fee_from_someone_elses_dollars_is_refused() {
    let mut w = opened();
    let (k, _) = newcomer(&mut w.svm, 0);
    let (_, theirs) = newcomer(&mut w.svm, 100);
    let err = try_pay_fee(&mut w.svm, &k, theirs, DOLLAR).unwrap_err();
    assert!(err.contains("ConstraintTokenOwner"), "{err}");
}

/// Pass an account of the caller's own where one of the pool's belongs.
fn swapped(ix: Instruction, index: usize, mint: Address, w: &mut World, who: &Keypair) -> String {
    let look_alike = Address::new_unique();
    put_token_account(&mut w.svm, look_alike, mint, who.pubkey(), u64::MAX / 4);
    let mut ix = ix;
    ix.accounts[index].pubkey = look_alike;
    w.svm.expire_blockhash();
    send(&mut w.svm, ix, &[who]).unwrap_err()
}

#[test]
fn look_alike_pool_accounts_are_refused_when_paying_a_fee() {
    for (index, mint) in [(4, usd()), (5, pda(&[COIN_SEED])), (7, usd()), (8, pda(&[COIN_SEED]))] {
        let mut w = opened();
        let (k, from) = newcomer(&mut w.svm, 100);
        let ix = pay_fee_ix(k.pubkey(), usd(), from, DOLLAR);
        let err = swapped(ix, index, mint, &mut w, &k);
        assert!(err.contains("ConstraintSeeds"), "account {index}: {err}");
    }
}

#[test]
fn look_alike_pool_accounts_are_refused_when_settling() {
    for (index, mint) in [(3, usd()), (4, pda(&[COIN_SEED])), (5, usd()), (6, pda(&[COIN_SEED]))] {
        let mut w = opened();
        let (k, _) = newcomer(&mut w.svm, 0);
        let ix = settle_ix(k.pubkey());
        let err = swapped(ix, index, mint, &mut w, &k);
        assert!(err.contains("ConstraintSeeds"), "account {index}: {err}");
    }
}

#[test]
fn nobody_can_take_the_networks_coin() {
    let mut w = opened();
    let (k, from) = newcomer(&mut w.svm, 100);
    try_pay_fee(&mut w.svm, &k, from, DOLLAR).unwrap();
    let held = network_coins(&w.svm);
    assert!(held > 0);

    let mine = Address::new_unique();
    put_token_account(&mut w.svm, mine, pda(&[COIN_SEED]), k.pubkey(), 0);
    let ix = spl_token_interface::instruction::transfer(&token_program(), &pda(&[NETWORK_SEED]), &mine, &k.pubkey(), &[], 1)
        .unwrap();
    w.svm.expire_blockhash();
    let err = send(&mut w.svm, ix, &[&k]).unwrap_err();
    assert!(err.contains("owner does not match"), "{err}");
    assert_eq!(network_coins(&w.svm), held);
}

#[test]
fn nobody_can_burn_the_pools_coin_themselves() {
    let mut w = opened();
    let (k, _) = newcomer(&mut w.svm, 0);
    let ix = spl_token_interface::instruction::burn(&token_program(), &pda(&[POOL_COIN_SEED]), &pda(&[COIN_SEED]), &k.pubkey(), &[], 1)
        .unwrap();
    w.svm.expire_blockhash();
    let err = send(&mut w.svm, ix, &[&k]).unwrap_err();
    assert!(err.contains("owner does not match"), "{err}");
}
