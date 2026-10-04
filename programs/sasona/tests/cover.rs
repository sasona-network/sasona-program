//! The cover: buyers paid back out of it, guarantees asked back and released,
//! and the move of guarantees made before it existed. Run against the
//! compiled program.

mod common;
use anchor_lang::AccountSerialize;
use common::*;

/// Opened, with a second depositor.
fn two_depositors() -> (World, Keypair, Address) {
    let mut w = world_unverified();
    open(&mut w, 1_220 * DOLLAR);
    let (k, from) = newcomer(&mut w.svm, 1_000);
    try_deposit(&mut w.svm, &k, from, 400 * DOLLAR).unwrap();
    (w, k, from)
}

fn coins_per_share_e12(svm: &LiteSVM) -> u128 {
    let c = cover(svm);
    c.coins as u128 * 1_000_000_000_000 / c.shares as u128
}

// ------------------------------------------------- paying a buyer back

// A buyer is paid back from the cover by a chargeback (part 7). Here the
// chargeback is left with no replay for 7 days, so the cover pays the price
// and nothing else, and the arithmetic of a payout is all that is tested.
// A member's room to insure is about $2 of stake, which bounds the amounts.

#[test]
fn a_payout_pays_the_buyer_and_the_cover_carries_it() {
    let (mut w, _, _) = two_depositors();
    let dollars = DOLLAR + 250_000;
    let (purchase, buyer) = charged_back(&mut w, dollars);
    let buyer_usd = ata(buyer.pubkey(), usd());
    let before = pool(&w.svm);
    let cover_before = cover(&w.svm);
    let supply = mint_state(&w.svm, pda(&[COIN_SEED])).supply;

    let ix = settle_chargeback_ix(&mut w.svm, purchase);
    try_ix(&mut w.svm, ix, &buyer).unwrap();

    let burn = (dollars as u128 * before.coin_reserve as u128).div_ceil(before.usd_reserve as u128) as u64;
    let after = pool(&w.svm);
    assert_eq!(token_balance(&w.svm, buyer_usd), dollars, "the buyer is paid in dollars");
    assert_eq!(after.usd_reserve, before.usd_reserve - dollars);
    assert_eq!(after.coin_reserve, before.coin_reserve - burn);
    assert_eq!(cover(&w.svm).coins, cover_before.coins - burn, "the cover paid");
    assert_eq!(cover(&w.svm).shares, cover_before.shares, "nobody's shares changed");
    assert_eq!(mint_state(&w.svm, pda(&[COIN_SEED])).supply, supply - 2 * burn);
    // The price did not fall.
    assert!(after.usd_reserve as u128 * before.coin_reserve as u128 >= before.usd_reserve as u128 * after.coin_reserve as u128);
    assert_books_balance(&w.svm);
}

#[test]
fn a_payout_after_the_price_has_moved_still_cannot_lower_it() {
    // At the opening price every payout divides exactly, so nothing rounds.
    // After a fee it does not, and the pool must give up at least the coins
    // behind the dollars.
    let (mut w, k, from) = two_depositors();
    try_pay_fee(&mut w.svm, &k, from, 3 * DOLLAR + 7).unwrap();
    for dollars in [300_007, 400_001, 299_999] {
        let (purchase, buyer) = charged_back(&mut w, dollars);
        let before = pool(&w.svm);
        assert_ne!((dollars as u128 * before.coin_reserve as u128) % before.usd_reserve as u128, 0,
                   "this amount has to round, or it tests nothing");
        let ix = settle_chargeback_ix(&mut w.svm, purchase);
        try_ix(&mut w.svm, ix, &buyer).unwrap();
        let after = pool(&w.svm);
        assert!(after.usd_reserve as u128 * before.coin_reserve as u128
            >= before.usd_reserve as u128 * after.coin_reserve as u128);
        assert_books_balance(&w.svm);
    }
}

#[test]
fn new_shares_are_never_worth_more_than_the_coins_paid_for_them() {
    let mut x: u64 = 0x5eed_5eed_5eed_5eed;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut rounded = 0;
    for _ in 0..100_000 {
        let cover_coins = 1 + next() % (1u64 << 40);
        let cover_shares = 1 + next() % (1u64 << 44);
        let coins = 1 + next() % (1u64 << 30);
        let shares = shares_for(coins, cover_shares, cover_coins).unwrap();
        let worth = shares as u128 * cover_coins as u128;
        let paid = coins as u128 * cover_shares as u128;
        assert!(worth <= paid, "coins {coins} cover {cover_coins}/{cover_shares}");
        if worth < paid {
            rounded += 1;
        }
    }
    assert!(rounded > 50_000, "only {rounded} cases rounded");
}

#[test]
fn a_payout_falls_on_every_share_alike() {
    let (mut w, k, _) = two_depositors();
    let opener = w.depositor.pubkey();
    let worth = |svm: &LiteSVM, who: Address| {
        let c = cover(svm);
        coins_for_shares(shares_of(svm, who), c.shares, c.coins).unwrap()
    };
    let (purchase, buyer) = charged_back(&mut w, DOLLAR + 500_000);
    let (a0, b0) = (worth(&w.svm, opener), worth(&w.svm, k.pubkey()));
    let ix = settle_chargeback_ix(&mut w.svm, purchase);
    try_ix(&mut w.svm, ix, &buyer).unwrap();
    let (a1, b1) = (worth(&w.svm, opener), worth(&w.svm, k.pubkey()));
    // Both lost the same fraction, to within a unit of rounding.
    let lhs = a1 as u128 * b0 as u128;
    let rhs = b1 as u128 * a0 as u128;
    assert!(lhs.abs_diff(rhs) <= a0.max(b0) as u128, "{a0}->{a1} vs {b0}->{b1}");
    assert!(a1 < a0 && b1 < b0);
}

/// Overwrite the cover's record, keeping the vault as it is.
fn set_cover(svm: &mut LiteSVM, coins: u64, shares: u64) {
    let c: Cover = read(svm, pda(&[COVER_SEED]));
    let mut data = Vec::new();
    Cover { bump: c.bump, shares, coins }.try_serialize(&mut data).unwrap();
    overwrite(svm, pda(&[COVER_SEED]), &data);
}

#[test]
fn a_payout_larger_than_the_cover_waits() {
    let (mut w, _, _) = two_depositors();
    let (purchase, buyer) = charged_back(&mut w, DOLLAR);
    // A cover with less in it than this payout would burn.
    set_cover(&mut w.svm, 1_000, 1_000);
    let ix = settle_chargeback_ix(&mut w.svm, purchase);
    let err = try_ix(&mut w.svm, ix, &buyer).unwrap_err();
    assert!(err.contains("MoreThanIsThere"), "{err}");
    let c = chargeback(&w.svm, purchase);
    assert_eq!(c.state, sasona::CHARGEBACK_OPEN, "the chargeback waits, and can be settled later");
}

#[test]
fn a_payout_that_would_thin_the_cover_too_far_waits() {
    let (mut w, _, _) = two_depositors();
    let (purchase, buyer) = charged_back(&mut w, DOLLAR);
    let p = pool(&w.svm);
    let burn = (DOLLAR as u128 * p.coin_reserve as u128).div_ceil(p.usd_reserve as u128) as u64;
    let coins = cover(&w.svm).coins;
    // As many shares as leave exactly one coin unit per thousand after the burn,
    // plus one: the payout would thin it past that.
    set_cover(&mut w.svm, coins, (coins - burn) * MAX_SHARES_PER_COIN + 1);
    let ix = settle_chargeback_ix(&mut w.svm, purchase);
    let err = try_ix(&mut w.svm, ix, &buyer).unwrap_err();
    assert!(err.contains("CoverTooThin"), "{err}");
    set_cover(&mut w.svm, coins, (coins - burn) * MAX_SHARES_PER_COIN);
    let ix = settle_chargeback_ix(&mut w.svm, purchase);
    try_ix(&mut w.svm, ix, &buyer).unwrap();
}

#[test]
fn a_payout_goes_only_to_the_buyer_and_in_dollars() {
    let (mut w, k, _) = two_depositors();
    let (purchase, buyer) = charged_back(&mut w, DOLLAR);
    // Another token, into an account the buyer holds.
    let other = Address::new_unique();
    put_mint(&mut w.svm, other, k.pubkey(), 6);
    let wrong = Address::new_unique();
    put_token_account(&mut w.svm, wrong, other, buyer.pubkey(), 0);
    let mut ix = settle_chargeback_ix(&mut w.svm, purchase);
    ix.accounts[12].pubkey = wrong;
    let err = try_ix(&mut w.svm, ix, &buyer).unwrap_err();
    assert!(err.contains("ConstraintTokenMint"), "{err}");
    // The pool's own dollar account, or the fee account, in place of the buyer's.
    for own in [pda(&[POOL_USD_SEED]), pda(&[FEES_SEED])] {
        let mut ix = settle_chargeback_ix(&mut w.svm, purchase);
        ix.accounts[12].pubkey = own;
        let err = try_ix(&mut w.svm, ix, &buyer).unwrap_err();
        assert!(err.contains("NotTheOwner") || err.contains("ConstraintDuplicateMutableAccount"), "{err}");
    }
}

#[test]
fn a_look_alike_cover_vault_is_refused() {
    let (mut w, k, _) = two_depositors();
    let (purchase, buyer) = charged_back(&mut w, DOLLAR);
    let look_alike = Address::new_unique();
    put_token_account(&mut w.svm, look_alike, pda(&[COIN_SEED]), k.pubkey(), u64::MAX / 4);
    let mut ix = settle_chargeback_ix(&mut w.svm, purchase);
    ix.accounts[10].pubkey = look_alike;
    let err = try_ix(&mut w.svm, ix, &buyer).unwrap_err();
    assert!(err.contains("ConstraintSeeds"), "{err}");
}

#[test]
fn depositing_after_a_payout_does_not_dilute_anyone() {
    let (mut w, k, from) = two_depositors();
    pay_back(&mut w, DOLLAR + 500_000).unwrap();
    let per_share = coins_per_share_e12(&w.svm);
    try_deposit(&mut w.svm, &k, from, 200 * DOLLAR).unwrap();
    // New shares were bought at the lower rate, so a share is worth the same.
    assert!(coins_per_share_e12(&w.svm) >= per_share);
    assert!(coins_per_share_e12(&w.svm) - per_share <= 1_000_000_000_000 / 1_000);
    assert_books_balance(&w.svm);
}

/// A covered purchase of `dollars`, charged back, with its 7 days for a
/// replay gone by: ready to settle. The buyer's dollar account is emptied,
/// so it ends holding only what comes back.
fn charged_back(w: &mut World, dollars: u64) -> (Address, Keypair) {
    // A service of its own each time keeps every chargeback the first on its
    // service in 30 days, and so free of deposits.
    let slot = w.svm.get_sysvar::<solana_clock::Clock>().slot + 1_000;
    let service = format!("https://paid-back.example/run/{slot}");
    let reading = insured_reading(w, &service, 100, slot);
    let buyer = new_buyer(&mut w.svm, 2 * dollars);
    let usd_acc = ata(buyer.pubkey(), usd());
    try_with(&mut w.svm, |s| buy_ix(s, buyer.pubkey(), usd_acc, reading, 1, dollars), &buyer).unwrap();
    let purchase = purchase_address(buyer.pubkey(), 1);
    try_with(&mut w.svm, |s| charge_back_ix(s, buyer.pubkey(), usd_acc, purchase), &buyer).unwrap();
    put_token_account(&mut w.svm, usd_acc, usd(), buyer.pubkey(), 0);
    let deposit = chargeback(&w.svm, purchase).deposit;
    assert_eq!(deposit, 0, "the first chargeback on a service in 30 days needs no deposit");
    days_pass(&mut w.svm, 8);
    (purchase, buyer)
}

// ---------------------------------------------------------------- release

#[test]
fn a_guarantee_comes_back_after_the_notice_and_not_before() {
    let (mut w, k, _) = two_depositors();
    let shares = shares_of(&w.svm, k.pubkey());
    try_ix(&mut w.svm, request_ix(k.pubkey(), shares), &k).unwrap();
    assert_eq!(shares_of(&w.svm, k.pubkey()), 0);

    days_pass(&mut w.svm, 44);
    let err = try_ix(&mut w.svm, release_ix(k.pubkey()), &k).unwrap_err();
    assert!(err.contains("NoticeNotOver"), "{err}");

    days_pass(&mut w.svm, 2);
    let c = cover(&w.svm);
    let expected = coins_for_shares(shares, c.shares, c.coins).unwrap();
    let held = token_balance(&w.svm, ata(k.pubkey(), pda(&[COIN_SEED])));
    try_ix(&mut w.svm, release_ix(k.pubkey()), &k).unwrap();
    assert_eq!(token_balance(&w.svm, ata(k.pubkey(), pda(&[COIN_SEED]))), held + expected);
    assert!(w.svm.get_account(&pda(&[EXIT_SEED, k.pubkey().as_ref()])).map(|a| a.lamports == 0).unwrap_or(true),
            "the exit record is closed");
    assert_eq!(cover(&w.svm).shares, c.shares - shares);
    assert_books_balance(&w.svm);
}

#[test]
fn a_payout_during_the_notice_still_lands_on_the_leaver() {
    let (mut w, k, _) = two_depositors();
    let shares = shares_of(&w.svm, k.pubkey());
    try_ix(&mut w.svm, request_ix(k.pubkey(), shares), &k).unwrap();
    let c = cover(&w.svm);
    let before_claim = coins_for_shares(shares, c.shares, c.coins).unwrap();

    pay_back(&mut w, DOLLAR + 500_000).unwrap();

    days_pass(&mut w.svm, 46);
    let held = token_balance(&w.svm, ata(k.pubkey(), pda(&[COIN_SEED])));
    try_ix(&mut w.svm, release_ix(k.pubkey()), &k).unwrap();
    let got = token_balance(&w.svm, ata(k.pubkey(), pda(&[COIN_SEED]))) - held;
    assert!(got < before_claim, "leaving did not dodge the claim: {got} vs {before_claim}");
}

#[test]
fn asking_again_restarts_the_notice() {
    let (mut w, k, _) = two_depositors();
    let shares = shares_of(&w.svm, k.pubkey());
    try_ix(&mut w.svm, request_ix(k.pubkey(), shares / 2), &k).unwrap();
    days_pass(&mut w.svm, 20);
    try_ix(&mut w.svm, request_ix(k.pubkey(), shares / 4), &k).unwrap();
    days_pass(&mut w.svm, 20);
    let err = try_ix(&mut w.svm, release_ix(k.pubkey()), &k).unwrap_err();
    assert!(err.contains("NoticeNotOver"), "{err}");
    let e: Exit = read(&w.svm, pda(&[EXIT_SEED, k.pubkey().as_ref()]));
    assert_eq!(e.shares, shares / 2 + shares / 4);
}

#[test]
fn asking_for_more_than_you_hold_is_refused() {
    let (mut w, k, _) = two_depositors();
    let shares = shares_of(&w.svm, k.pubkey());
    let err = try_ix(&mut w.svm, request_ix(k.pubkey(), shares + 1), &k).unwrap_err();
    assert!(err.contains("MoreThanIsThere"), "{err}");
}

#[test]
fn nobody_can_ask_for_someone_elses_guarantee() {
    let (mut w, k, _) = two_depositors();
    let opener = w.depositor.pubkey();
    let mut ix = request_ix(k.pubkey(), 1);
    ix.accounts[1].pubkey = pda(&[GUARANTEE_SEED, opener.as_ref()]);
    let err = try_ix(&mut w.svm, ix, &k).unwrap_err();
    assert!(err.contains("ConstraintSeeds"), "{err}");
}

#[test]
fn nobody_can_release_someone_elses_exit() {
    let (mut w, k, _) = two_depositors();
    let shares = shares_of(&w.svm, k.pubkey());
    try_ix(&mut w.svm, request_ix(k.pubkey(), shares), &k).unwrap();
    days_pass(&mut w.svm, 46);
    let (thief, _) = newcomer(&mut w.svm, 0);
    let mut ix = release_ix(thief.pubkey());
    ix.accounts[5].pubkey = pda(&[EXIT_SEED, k.pubkey().as_ref()]);
    let err = try_ix(&mut w.svm, ix, &thief).unwrap_err();
    assert!(err.contains("ConstraintSeeds"), "{err}");
}

#[test]
fn the_last_one_out_takes_every_coin_left() {
    let mut w = world_unverified();
    open(&mut w, 1_220 * DOLLAR);
    pay_back(&mut w, DOLLAR + 3).unwrap();
    let d = w.depositor.insecure_clone();
    let shares = shares_of(&w.svm, d.pubkey());
    try_ix(&mut w.svm, request_ix(d.pubkey(), shares), &d).unwrap();
    days_pass(&mut w.svm, 46);
    try_ix(&mut w.svm, release_ix(d.pubkey()), &d).unwrap();
    let c = cover(&w.svm);
    assert_eq!((c.shares, c.coins), (0, 0));
    assert_eq!(token_balance(&w.svm, pda(&[COVER_VAULT_SEED])), 0);
    assert_books_balance(&w.svm);
}

// ------------------------------------------------- guarantees made before

/// Put the opener back in the state devnet is in: their guarantee in its own
/// vault, the record counting coins, and the cover empty.
fn as_before_the_cover(w: &mut World) -> u64 {
    let d = w.depositor.pubkey();
    let coins = token_balance(&w.svm, pda(&[COVER_VAULT_SEED]));
    put_token_account(&mut w.svm, pda(&[VAULT_SEED, d.as_ref()]), pda(&[COIN_SEED]), pda(&[POOL_SEED]), coins);
    put_token_account(&mut w.svm, pda(&[COVER_VAULT_SEED]), pda(&[COIN_SEED]), pda(&[POOL_SEED]), 0);

    let c: Cover = read(&w.svm, pda(&[COVER_SEED]));
    let mut data = Vec::new();
    Cover { bump: c.bump, shares: 0, coins: 0 }.try_serialize(&mut data).unwrap();
    overwrite(&mut w.svm, pda(&[COVER_SEED]), &data);

    let g: Guarantee = read(&w.svm, pda(&[GUARANTEE_SEED, d.as_ref()]));
    let mut data = Vec::new();
    Guarantee { owner: g.owner, shares: coins, bump: g.bump }.try_serialize(&mut data).unwrap();
    overwrite(&mut w.svm, pda(&[GUARANTEE_SEED, d.as_ref()]), &data);
    coins
}

fn overwrite(svm: &mut LiteSVM, at: Address, data: &[u8]) {
    let mut acc = svm.get_account(&at).unwrap();
    acc.data[..data.len()].copy_from_slice(data);
    svm.set_account(at, acc).unwrap();
}

#[test]
fn an_old_guarantee_joins_the_cover() {
    let mut w = world_unverified();
    open(&mut w, 1_220 * DOLLAR);
    let coins = as_before_the_cover(&mut w);
    let d = w.depositor.insecure_clone();
    let lamports_before = w.svm.get_account(&d.pubkey()).unwrap().lamports;

    // Until it joins, the old guarantee blocks deposits and requests.
    let from = w.depositor_usd;
    let err = try_deposit(&mut w.svm, &d, from, 100 * DOLLAR).unwrap_err();
    assert!(err.contains("JoinCoverFirst"), "{err}");
    let err = try_ix(&mut w.svm, request_ix(d.pubkey(), 1), &d).unwrap_err();
    assert!(err.contains("JoinCoverFirst"), "{err}");

    let (anyone, _) = newcomer(&mut w.svm, 0);
    try_ix(&mut w.svm, join_ix(anyone.pubkey(), d.pubkey()), &anyone).unwrap();

    assert_eq!(cover(&w.svm).coins, coins);
    assert_eq!(shares_of(&w.svm, d.pubkey()), coins);
    assert_eq!(token_balance(&w.svm, pda(&[COVER_VAULT_SEED])), coins);
    assert!(w.svm.get_account(&pda(&[VAULT_SEED, d.pubkey().as_ref()])).map(|a| a.lamports == 0).unwrap_or(true),
            "the old vault is closed");
    assert!(w.svm.get_account(&d.pubkey()).unwrap().lamports > lamports_before, "its rent went back to the owner");
    assert_books_balance(&w.svm);

    try_deposit(&mut w.svm, &d, from, 100 * DOLLAR).unwrap();
    let err = try_ix(&mut w.svm, join_ix(anyone.pubkey(), d.pubkey()), &anyone).unwrap_err();
    assert!(err.contains("AccountNotInitialized") || err.contains("ConstraintSeeds"), "{err}");
}

#[test]
fn a_stray_coin_in_the_old_vault_cannot_block_the_move() {
    let mut w = world_unverified();
    open(&mut w, 1_220 * DOLLAR);
    let coins = as_before_the_cover(&mut w);
    let d = w.depositor.pubkey();
    // Someone sends the old vault one more coin unit, here from the
    // opener's own free coins so the supply still balances.
    let free = ata(d, pda(&[COIN_SEED]));
    let held = token_balance(&w.svm, free);
    put_token_account(&mut w.svm, free, pda(&[COIN_SEED]), d, held - 1);
    put_token_account(&mut w.svm, pda(&[VAULT_SEED, d.as_ref()]), pda(&[COIN_SEED]), pda(&[POOL_SEED]), coins + 1);

    let (anyone, _) = newcomer(&mut w.svm, 0);
    try_ix(&mut w.svm, join_ix(anyone.pubkey(), d), &anyone).unwrap();
    assert_eq!(shares_of(&w.svm, d), coins + 1);
    assert_books_balance(&w.svm);
}

#[test]
fn nobody_can_ask_back_a_guarantee_without_its_owners_signature() {
    // With signatures checked, as on devnet.
    let mut w = world();
    open(&mut w, 1_220 * DOLLAR);
    let owner = w.depositor.pubkey();
    let (thief, _) = newcomer(&mut w.svm, 0);
    w.svm.expire_blockhash();
    let msg = solana_message::Message::new_with_blockhash(&[request_ix(owner, 1)], Some(&thief.pubkey()), &w.svm.latest_blockhash());
    let mut tx = Transaction::new_unsigned(msg);
    tx.partial_sign(&[&thief], w.svm.latest_blockhash());
    assert!(w.svm.send_transaction(tx).is_err());
    assert_eq!(shares_of(&w.svm, owner), Slices::of(1_220 * DOLLAR).unwrap().guarantee * PRICE);
}

#[test]
fn joining_twice_or_with_the_wrong_owner_is_refused() {
    let mut w = world_unverified();
    open(&mut w, 1_220 * DOLLAR);
    as_before_the_cover(&mut w);
    let d = w.depositor.pubkey();
    let (anyone, _) = newcomer(&mut w.svm, 0);
    // The rent must go to the guarantee's owner and nobody else.
    let mut ix = join_ix(anyone.pubkey(), d);
    ix.accounts[5].pubkey = anyone.pubkey();
    let err = try_ix(&mut w.svm, ix, &anyone).unwrap_err();
    assert!(err.contains("ConstraintAddress"), "{err}");
}
