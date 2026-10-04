//! Purchases and chargebacks: a purchase covered by a quote, a chargeback
//! settled by a replay drawn for it, the cover paying the buyer, and the
//! member who insured it paying the cover back. Run against the compiled
//! program.

mod common;
use common::*;
use sasona::{
    canonical_question, Book, Purchase, CHALLENGE_WINDOW_SECONDS, CHARGEBACK_SETTLED, MEMBER_STAKE, MIN_COVERED_PRICE,
    PURCHASE_CLOSED, READ_WINDOW_SLOTS, REVEAL_WINDOW_SLOTS,
};

const SERVICE: &str = "https://sandbox.example.net/run/python";
const PRICE: u64 = DOLLAR;
const FEE: u64 = DOLLAR / 20;

fn usd_balance(svm: &LiteSVM, who: Address) -> u64 {
    token_balance(svm, ata(who, usd()))
}

struct Market {
    w: World,
    quoter: Keypair,
    reader: Keypair,
    reading: Address,
}

/// The depositor (membership 1, seat 1) has read SERVICE and quoted it at 150
/// bps; b holds membership 2 in seat 2, and so is the only one a replay can draw.
fn market() -> Market {
    let mut w = world_with_member();
    let quoter = w.depositor.insecure_clone();
    let reading = insured_reading(&mut w, SERVICE, 150, 1_000);
    let (reader, _) = new_member(&mut w.svm);
    usd_of(&mut w.svm, reader.pubkey());
    // b sat down at this slot; draws from later slots can draw b.
    w.svm.warp_to_slot(1_100);
    Market { w, quoter, reader, reading }
}

fn buy(m: &mut Market, buyer: &Keypair, id: u64, price: u64) -> Result<Address, String> {
    let usd_acc = ata(buyer.pubkey(), usd());
    let reading = m.reading;
    try_with(&mut m.w.svm, |s| buy_ix(s, buyer.pubkey(), usd_acc, reading, id, price), buyer)?;
    Ok(purchase_address(buyer.pubkey(), id))
}

fn charge(m: &mut Market, buyer: &Keypair, purchase: Address) -> Result<(), String> {
    let usd_acc = ata(buyer.pubkey(), usd());
    try_with(&mut m.w.svm, |s| charge_back_ix(s, buyer.pubkey(), usd_acc, purchase), buyer)
}

/// Record the current draw's entropy and have b replay with `verdict`.
fn replay(m: &mut Market, purchase: Address, verdict: u8) {
    let c = chargeback(&m.w.svm, purchase);
    let slot = c.draw_slot + 40;
    at_slot(&mut m.w.svm, slot, &recent(slot, &[]));
    let reader = m.reader.insecure_clone();
    // Recorded first, so the seats passed over before b's can be worked out
    // and shown.
    try_ix(&mut m.w.svm, record_draw_ix(purchase), &reader).unwrap();
    let nonce = [c.draw + 11; 16];
    let q = sha256(&canonical_question(&nonce));
    try_with(&mut m.w.svm, |s| commit_replay_ix(s, reader.pubkey(), purchase, SERVICE, q), &reader).unwrap();
    m.w.svm.warp_to_slot(slot + 1);
    let r = replay_address(chargeback_address(purchase), c.draw);
    try_ix(&mut m.w.svm, reveal_reading_ix(reader.pubkey(), r, nonce, [5u8; 32], verdict), &reader).unwrap();
}

fn settle(m: &mut Market, purchase: Address) -> Result<(), String> {
    let ix = settle_chargeback_ix(&mut m.w.svm, purchase);
    let anyone = m.reader.insecure_clone();
    try_ix(&mut m.w.svm, ix, &anyone)
}

#[test]
fn a_covered_purchase_pays_the_merchant_and_the_premium() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let merchant = merchant_usd(&mut m.w.svm);
    let merchant_before = token_balance(&m.w.svm, merchant);
    let quoter_before = usd_balance(&m.w.svm, m.quoter.pubkey());
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    assert_eq!(token_balance(&m.w.svm, merchant), merchant_before + PRICE);
    assert_eq!(usd_balance(&m.w.svm, m.quoter.pubkey()), quoter_before + PRICE * 150 / 10_000);
    assert_eq!(usd_balance(&m.w.svm, buyer.pubkey()), 10 * DOLLAR - PRICE - PRICE * 150 / 10_000);
    let pu: Purchase = read(&m.w.svm, purchase);
    assert_eq!((addr(pu.buyer), addr(pu.reading), pu.member, pu.price, pu.counted), (buyer.pubkey(), m.reading, 1, PRICE, PRICE + FEE));
    let b: Book = book(&m.w.svm, 1);
    assert_eq!((b.open_usd, b.owed_coins), (PRICE + FEE, 0));
}

#[test]
fn a_purchase_is_covered_only_on_its_terms() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 100 * DOLLAR);
    let err = buy(&mut m, &buyer, 1, MIN_COVERED_PRICE - 1).unwrap_err();
    assert!(err.contains("TooSmall"), "{err}");
    // More than the member's stake can stand behind: about $2 at this price.
    let err = buy(&mut m, &buyer, 1, 2 * DOLLAR).unwrap_err();
    assert!(err.contains("NoRoomToInsure"), "{err}");
    // Paid anywhere but where the reading says the service is paid.
    let usd_acc = ata(buyer.pubkey(), usd());
    let mut ix = buy_ix(&m.w.svm, buyer.pubkey(), usd_acc, m.reading, 1, PRICE);
    let own = usd_of(&mut m.w.svm, buyer.pubkey());
    ix.accounts[9].pubkey = own;
    let err = try_ix(&mut m.w.svm, ix, &buyer).unwrap_err();
    assert!(err.contains("NotThePayTo") || err.contains("ConstraintDuplicateMutableAccount"), "{err}");
    // A withdrawn quote covers nothing.
    let quoter = m.quoter.insecure_clone();
    let reading = m.reading;
    try_with(&mut m.w.svm, |s| set_quote_ix(s, quoter.pubkey(), reading, 0), &quoter).unwrap();
    let err = buy(&mut m, &buyer, 1, PRICE).unwrap_err();
    assert!(err.contains("NotInsured"), "{err}");
}

#[test]
fn a_reading_that_named_no_payee_covers_nothing() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let fp = [3u8; 32];
    w.svm.warp_to_slot(1_000);
    try_ix(&mut w.svm, open_round_ix(d.pubkey(), fp, 10, 2, sha256(&fp)), &d).unwrap();
    at_slot(&mut w.svm, 1_040, &recent(1_040, &[]));
    try_ix(&mut w.svm, reveal_ix(round_address(fp), d.pubkey(), fp), &d).unwrap();
    let q = sha256(&canonical_question(&[4u8; 16]));
    try_with(&mut w.svm, |s| commit_reading_ix(s, d.pubkey(), round_address(fp), SERVICE, q), &d).unwrap();
    let reading = reading_address(round_address(fp), SERVICE);
    w.svm.warp_to_slot(1_041);
    try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), reading, [4u8; 16], [1u8; 32], 1), &d).unwrap();
    merchant_usd(&mut w.svm);
    usd_of(&mut w.svm, d.pubkey());
    try_with(&mut w.svm, |s| set_quote_ix(s, d.pubkey(), reading, 100), &d).unwrap();
    let buyer = new_buyer(&mut w.svm, 10 * DOLLAR);
    let usd_acc = ata(buyer.pubkey(), usd());
    let err = try_with(&mut w.svm, |s| buy_ix(s, buyer.pubkey(), usd_acc, reading, 1, PRICE), &buyer).unwrap_err();
    // No account can be paid there: the address is all zeros.
    assert!(err.contains("NoPayTo") || err.contains("NotThePayTo"), "{err}");
}

#[test]
fn a_quote_covers_nothing_once_its_reading_is_thirty_days_old() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    days_pass(&mut m.w.svm, 31);
    let err = buy(&mut m, &buyer, 1, PRICE).unwrap_err();
    assert!(err.contains("WindowClosed"), "{err}");
}

#[test]
fn a_purchase_closes_after_seven_days_and_gives_the_room_back() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    let anyone = m.reader.insecure_clone();
    days_pass(&mut m.w.svm, 7);
    let err = try_with(&mut m.w.svm, |s| close_purchase_ix(s, purchase), &anyone).unwrap_err();
    assert!(err.contains("NotLapsedYet"), "{err}");
    days_pass(&mut m.w.svm, 1);
    try_with(&mut m.w.svm, |s| close_purchase_ix(s, purchase), &anyone).unwrap();
    let pu: Purchase = read(&m.w.svm, purchase);
    assert_eq!((pu.state, book(&m.w.svm, 1).open_usd), (PURCHASE_CLOSED, 0));
    let err = charge(&mut m, &buyer, purchase).unwrap_err();
    assert!(err.contains("NotOpen"), "{err}");
}

#[test]
fn a_member_still_insuring_cannot_take_their_stake_back() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    buy(&mut m, &buyer, 1, PRICE).unwrap();
    let quoter = m.quoter.insecure_clone();
    try_with(&mut m.w.svm, |s| ask_to_leave_ix(s, quoter.pubkey(), 1), &quoter).unwrap();
    days_pass(&mut m.w.svm, 46);
    let err = try_ix(&mut m.w.svm, leave_ix(quoter.pubkey(), 1), &quoter).unwrap_err();
    assert!(err.contains("StillInsuring"), "{err}");
}

#[test]
fn the_first_chargeback_on_a_service_in_thirty_days_is_free() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let one = buy(&mut m, &buyer, 1, PRICE).unwrap();
    let two = buy(&mut m, &buyer, 2, 300_000).unwrap();
    let before = usd_balance(&m.w.svm, buyer.pubkey());
    charge(&mut m, &buyer, one).unwrap();
    assert_eq!((chargeback(&m.w.svm, one).deposit, usd_balance(&m.w.svm, buyer.pubkey())), (0, before));
    charge(&mut m, &buyer, two).unwrap();
    assert_eq!(chargeback(&m.w.svm, two).deposit, 15_000);
    assert_eq!(token_balance(&m.w.svm, pda(&[sasona::ESCROW_SEED])), 15_000);
    let err = charge(&mut m, &buyer, one).unwrap_err();
    assert!(err.contains("NotOpen") || err.contains("already in use"), "{err}");
}

#[test]
fn only_the_buyer_charges_back_and_only_within_seven_days() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    let other = new_buyer(&mut m.w.svm, DOLLAR);
    let other_usd = ata(other.pubkey(), usd());
    let err = try_with(&mut m.w.svm, |s| charge_back_ix(s, other.pubkey(), other_usd, purchase), &other).unwrap_err();
    assert!(err.contains("NotTheOwner"), "{err}");
    days_pass(&mut m.w.svm, 8);
    let err = charge(&mut m, &buyer, purchase).unwrap_err();
    assert!(err.contains("WindowClosed"), "{err}");
}

#[test]
fn a_failed_replay_pays_the_buyer_and_the_replayer_and_the_insurer_owes_it() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let first = buy(&mut m, &buyer, 1, 300_000).unwrap();
    charge(&mut m, &buyer, first).unwrap(); // spends the free chargeback
    let purchase = buy(&mut m, &buyer, 2, PRICE).unwrap();
    charge(&mut m, &buyer, purchase).unwrap();
    let deposit = chargeback(&m.w.svm, purchase).deposit;
    assert_eq!(deposit, FEE);
    replay(&mut m, purchase, 2);

    let buyer_before = usd_balance(&m.w.svm, buyer.pubkey());
    let reader_before = usd_balance(&m.w.svm, m.reader.pubkey());
    let cover_before = cover(&m.w.svm).coins;
    let open_before = book(&m.w.svm, 1).open_usd;
    settle(&mut m, purchase).unwrap();
    assert_eq!(usd_balance(&m.w.svm, buyer.pubkey()), buyer_before + PRICE + deposit, "the price, and the deposit back");
    assert_eq!(usd_balance(&m.w.svm, m.reader.pubkey()), reader_before + FEE, "the replayer is paid");
    let c = chargeback(&m.w.svm, purchase);
    assert_eq!(c.state, CHARGEBACK_SETTLED);
    assert_eq!(c.owed_coins, cover_before - cover(&m.w.svm).coins, "the member owes the cover what it burned");
    let b = book(&m.w.svm, 1);
    assert_eq!((b.open_usd, b.owed_coins), (open_before - (PRICE + FEE), c.owed_coins));
    assert_books_balance(&m.w.svm);
    let err = settle(&mut m, purchase).unwrap_err();
    assert!(err.contains("ChallengeClosed"), "{err}");
}

#[test]
fn a_replay_that_delivers_pays_the_replayer_from_the_deposit() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let first = buy(&mut m, &buyer, 1, 300_000).unwrap();
    charge(&mut m, &buyer, first).unwrap();
    let purchase = buy(&mut m, &buyer, 2, PRICE).unwrap();
    charge(&mut m, &buyer, purchase).unwrap();
    replay(&mut m, purchase, 1);
    let buyer_before = usd_balance(&m.w.svm, buyer.pubkey());
    let reader_before = usd_balance(&m.w.svm, m.reader.pubkey());
    let cover_before = cover(&m.w.svm).coins;
    settle(&mut m, purchase).unwrap();
    assert_eq!(usd_balance(&m.w.svm, buyer.pubkey()), buyer_before, "not paid back");
    assert_eq!(usd_balance(&m.w.svm, m.reader.pubkey()), reader_before + FEE, "the deposit pays the replayer");
    assert_eq!(cover(&m.w.svm).coins, cover_before, "the cover paid nothing");
    assert_eq!(chargeback(&m.w.svm, purchase).owed_coins, 0);
}

#[test]
fn a_free_chargeback_that_delivers_costs_the_insurer_the_fee() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    charge(&mut m, &buyer, purchase).unwrap();
    replay(&mut m, purchase, 1);
    let reader_before = usd_balance(&m.w.svm, m.reader.pubkey());
    settle(&mut m, purchase).unwrap();
    assert_eq!(usd_balance(&m.w.svm, m.reader.pubkey()), reader_before + FEE);
    assert!(chargeback(&m.w.svm, purchase).owed_coins > 0);
}

#[test]
fn the_draw_passes_over_the_buyers_and_the_insurers_seats() {
    // With only the quoter and the buyer seated, nobody can replay.
    let mut w = world_with_member();
    let quoter = w.depositor.insecure_clone();
    let reading = insured_reading(&mut w, SERVICE, 150, 1_000);
    let (buyer, _) = new_member(&mut w.svm);
    put_token_account(&mut w.svm, ata(buyer.pubkey(), usd()), usd(), buyer.pubkey(), 10 * DOLLAR);
    w.svm.warp_to_slot(1_100);
    let mut m = Market { w, quoter, reader: buyer.insecure_clone(), reading };
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    charge(&mut m, &buyer, purchase).unwrap();
    let c = chargeback(&m.w.svm, purchase);
    at_slot(&mut m.w.svm, c.draw_slot + 40, &recent(c.draw_slot + 40, &[]));
    try_ix(&mut m.w.svm, record_draw_ix(purchase), &buyer).unwrap();
    assert_eq!(replay_drawn_for(&m.w.svm, purchase).0, None);
    let q = sha256(&canonical_question(&[1u8; 16]));
    let err = try_with(&mut m.w.svm, |s| commit_replay_ix(s, buyer.pubkey(), purchase, SERVICE, q), &buyer).unwrap_err();
    assert!(err.contains("NotDrawn") || err.contains("NotSkippable"), "{err}");
    // Claiming the buyer's seat with every seat before it shown is refused
    // too: the buyer's seat never qualifies.
    let err = try_with(&mut m.w.svm, |s| claim_replay_ix(s, purchase, 2, SERVICE, q), &buyer).unwrap_err();
    assert!(err.contains("NotDrawn"), "{err}");
}

#[test]
fn a_replay_is_of_the_service_bought_by_the_member_drawn() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    charge(&mut m, &buyer, purchase).unwrap();
    let c = chargeback(&m.w.svm, purchase);
    at_slot(&mut m.w.svm, c.draw_slot + 40, &recent(c.draw_slot + 40, &[]));
    let q = sha256(&canonical_question(&[1u8; 16]));
    let reader = m.reader.insecure_clone();
    try_ix(&mut m.w.svm, record_draw_ix(purchase), &reader).unwrap();
    let err = try_with(&mut m.w.svm, |s| commit_replay_ix(s, reader.pubkey(), purchase, "https://other.example/x", q), &reader)
        .unwrap_err();
    assert!(err.contains("NotTheSameService"), "{err}");
    // The quoter, who is passed over, cannot replay their own case.
    let quoter = m.quoter.insecure_clone();
    let err = try_with(&mut m.w.svm, |s| commit_replay_ix(s, quoter.pubkey(), purchase, SERVICE, q), &quoter).unwrap_err();
    assert!(err.contains("NotDrawn") || err.contains("NotSkippable"), "{err}");
    try_with(&mut m.w.svm, |s| commit_replay_ix(s, reader.pubkey(), purchase, SERVICE, q), &reader).unwrap();
}

#[test]
fn a_replay_is_committed_within_the_hour() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    charge(&mut m, &buyer, purchase).unwrap();
    let c = chargeback(&m.w.svm, purchase);
    at_slot(&mut m.w.svm, c.draw_slot + 40, &recent(c.draw_slot + 40, &[]));
    let reader = m.reader.insecure_clone();
    try_ix(&mut m.w.svm, record_draw_ix(purchase), &reader).unwrap();
    let entropy_slot = chargeback(&m.w.svm, purchase).entropy_slot;
    m.w.svm.warp_to_slot(entropy_slot + READ_WINDOW_SLOTS + 1);
    let q = sha256(&canonical_question(&[1u8; 16]));
    let err = try_with(&mut m.w.svm, |s| commit_replay_ix(s, reader.pubkey(), purchase, SERVICE, q), &reader).unwrap_err();
    assert!(err.contains("WindowClosed"), "{err}");
}

#[test]
fn a_declined_draw_is_passed_and_its_seat_passed_over_after() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    charge(&mut m, &buyer, purchase).unwrap();
    let c = chargeback(&m.w.svm, purchase);
    at_slot(&mut m.w.svm, c.draw_slot + 40, &recent(c.draw_slot + 40, &[]));
    try_ix(&mut m.w.svm, record_draw_ix(purchase), &buyer).unwrap();
    let anyone = m.reader.insecure_clone();
    let err = try_with(&mut m.w.svm, |s| pass_draw_ix(s, purchase), &anyone).unwrap_err();
    assert!(err.contains("TooEarly"), "the hour is not over: {err}");
    let entropy_slot = chargeback(&m.w.svm, purchase).entropy_slot;
    m.w.svm.warp_to_slot(entropy_slot + READ_WINDOW_SLOTS + 1);
    try_with(&mut m.w.svm, |s| pass_draw_ix(s, purchase), &anyone).unwrap();
    let c = chargeback(&m.w.svm, purchase);
    assert_eq!((c.counted_draws, c.draw, c.declined[0]), (1, 1, 2), "membership 2 declined");
    // The next draw passes over the one that declined: nobody is left.
    let slot = c.draw_slot + 40;
    at_slot(&mut m.w.svm, slot, &recent(slot, &[]));
    try_ix(&mut m.w.svm, record_draw_ix(purchase), &buyer).unwrap();
    assert_eq!(replay_drawn_for(&m.w.svm, purchase).0, None);
    let q = sha256(&canonical_question(&[1u8; 16]));
    let reader = m.reader.insecure_clone();
    let err = try_with(&mut m.w.svm, |s| claim_replay_ix(s, purchase, 2, SERVICE, q), &reader).unwrap_err();
    assert!(err.contains("NotDrawn"), "{err}");
}

#[test]
fn a_draw_whose_entropy_was_never_recorded_still_counts() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    charge(&mut m, &buyer, purchase).unwrap();
    let c = chargeback(&m.w.svm, purchase);
    let anyone = m.reader.insecure_clone();
    // Still readable: the draw has to be recorded and used, not passed.
    at_slot(&mut m.w.svm, c.draw_slot + 40, &recent(c.draw_slot + 40, &[]));
    let err = try_with(&mut m.w.svm, |s| pass_draw_ix(s, purchase), &anyone).unwrap_err();
    assert!(err.contains("TooEarly"), "{err}");
    // Long gone from SlotHashes, and its hour over.
    let late = c.draw_slot + 32 + READ_WINDOW_SLOTS + 1;
    at_slot(&mut m.w.svm, late, &recent(late, &[]));
    try_with(&mut m.w.svm, |s| pass_draw_ix(s, purchase), &anyone).unwrap();
    let c = chargeback(&m.w.svm, purchase);
    assert_eq!((c.counted_draws, c.declined[0]), (1, 0), "it counts, its seat unknown");
}

#[test]
fn after_eight_passed_draws_the_buyer_is_paid_and_the_insurer_owes_it() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    charge(&mut m, &buyer, purchase).unwrap();
    let anyone = m.reader.insecure_clone();
    let err = settle(&mut m, purchase).unwrap_err();
    assert!(err.contains("NotLapsedYet"), "{err}");
    for _ in 0..8 {
        let c = chargeback(&m.w.svm, purchase);
        let late = c.draw_slot + 32 + READ_WINDOW_SLOTS + 1;
        at_slot(&mut m.w.svm, late, &recent(late, &[]));
        try_with(&mut m.w.svm, |s| pass_draw_ix(s, purchase), &anyone).unwrap();
    }
    assert_eq!(chargeback(&m.w.svm, purchase).counted_draws, 8);
    let err = try_with(&mut m.w.svm, |s| pass_draw_ix(s, purchase), &anyone).unwrap_err();
    assert!(err.contains("ChallengeClosed"), "{err}");
    let before = usd_balance(&m.w.svm, buyer.pubkey());
    settle(&mut m, purchase).unwrap();
    assert_eq!(usd_balance(&m.w.svm, buyer.pubkey()), before + PRICE);
    assert!(chargeback(&m.w.svm, purchase).owed_coins > 0);
}

#[test]
fn a_replay_committed_and_never_revealed_counts_as_declined() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    charge(&mut m, &buyer, purchase).unwrap();
    let c = chargeback(&m.w.svm, purchase);
    let slot = c.draw_slot + 40;
    at_slot(&mut m.w.svm, slot, &recent(slot, &[]));
    let reader = m.reader.insecure_clone();
    try_ix(&mut m.w.svm, record_draw_ix(purchase), &reader).unwrap();
    let q = sha256(&canonical_question(&[1u8; 16]));
    try_with(&mut m.w.svm, |s| commit_replay_ix(s, reader.pubkey(), purchase, SERVICE, q), &reader).unwrap();
    let entropy_slot = chargeback(&m.w.svm, purchase).entropy_slot;
    m.w.svm.warp_to_slot(entropy_slot + READ_WINDOW_SLOTS + 1);
    let err = try_with(&mut m.w.svm, |s| pass_draw_ix(s, purchase), &buyer).unwrap_err();
    assert!(err.contains("ReplayStands"), "it can still be revealed: {err}");
    m.w.svm.warp_to_slot(slot + REVEAL_WINDOW_SLOTS + 1);
    try_with(&mut m.w.svm, |s| pass_draw_ix(s, purchase), &buyer).unwrap();
    assert_eq!(chargeback(&m.w.svm, purchase).declined[0], 2);
}

#[test]
fn the_insurer_pays_the_cover_back_once_their_reading_can_no_longer_be_challenged() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    charge(&mut m, &buyer, purchase).unwrap();
    replay(&mut m, purchase, 3);
    settle(&mut m, purchase).unwrap();
    let owed = chargeback(&m.w.svm, purchase).owed_coins;
    assert!(owed > 0);
    let anyone = m.reader.insecure_clone();
    let err = try_with(&mut m.w.svm, |s| repay_cover_ix(s, purchase), &anyone).unwrap_err();
    assert!(err.contains("NotLapsedYet"), "{err}");
    // And the debt keeps the member from leaving meanwhile.
    let quoter = m.quoter.insecure_clone();
    try_with(&mut m.w.svm, |s| ask_to_leave_ix(s, quoter.pubkey(), 1), &quoter).unwrap();

    let r: sasona::Reading = read(&m.w.svm, m.reading);
    let now = m.w.svm.get_sysvar::<solana_clock::Clock>().unix_timestamp;
    days_pass(&mut m.w.svm, (r.reveal_time + CHALLENGE_WINDOW_SECONDS - now) / 86_400 + 1);
    let cover_before = cover(&m.w.svm).coins;
    try_with(&mut m.w.svm, |s| repay_cover_ix(s, purchase), &anyone).unwrap();
    assert_eq!(cover(&m.w.svm).coins, cover_before + owed);
    assert_eq!(member(&m.w.svm, 1).stake, MEMBER_STAKE - owed);
    assert_eq!(book(&m.w.svm, 1).owed_coins, 0);
    assert_eq!(member(&m.w.svm, 1).seat, 0, "less than a whole stake is not a membership any more");
    assert_books_balance(&m.w.svm);
    let err = try_with(&mut m.w.svm, |s| repay_cover_ix(s, purchase), &anyone).unwrap_err();
    assert!(err.contains("NothingOwed"), "{err}");
    days_pass(&mut m.w.svm, 46);
    try_ix(&mut m.w.svm, leave_ix(quoter.pubkey(), 1), &quoter).unwrap();
}

#[test]
fn what_the_insurer_owes_takes_room_to_insure() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    charge(&mut m, &buyer, purchase).unwrap();
    replay(&mut m, purchase, 2);
    settle(&mut m, purchase).unwrap();
    // $1.05 was paid out of a stake worth about $2: a second dollar does not fit.
    let err = buy(&mut m, &buyer, 2, PRICE).unwrap_err();
    assert!(err.contains("NoRoomToInsure"), "{err}");
    buy(&mut m, &buyer, 2, 500_000).unwrap();
}

#[test]
fn a_member_whose_stake_was_taken_down_leaves_their_seat() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    charge(&mut m, &buyer, purchase).unwrap();
    replay(&mut m, purchase, 2);
    settle(&mut m, purchase).unwrap();
    let r: sasona::Reading = read(&m.w.svm, m.reading);
    let now = m.w.svm.get_sysvar::<solana_clock::Clock>().unix_timestamp;
    days_pass(&mut m.w.svm, (r.reveal_time + CHALLENGE_WINDOW_SECONDS - now) / 86_400 + 1);
    let anyone = m.reader.insecure_clone();
    try_with(&mut m.w.svm, |s| repay_cover_ix(s, purchase), &anyone).unwrap();
    let quoter = member(&m.w.svm, 1);
    assert_eq!((quoter.seat, quoter.state), (0, sasona::MEMBER_LEAVING));
    // It reads nothing more, and insures nothing more.
    let err = buy(&mut m, &buyer, 2, 300_000).unwrap_err();
    assert!(err.contains("NotActive") || err.contains("WindowClosed"), "{err}");
    assert_eq!(seated(&m.w.svm), 1, "b alone is seated");
}

#[test]
fn a_raised_quote_does_not_charge_a_buyer_who_saw_the_old_one() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let quoter = m.quoter.insecure_clone();
    let reading = m.reading;
    try_with(&mut m.w.svm, |s| set_quote_ix(s, quoter.pubkey(), reading, 10_000), &quoter).unwrap();
    let usd_acc = ata(buyer.pubkey(), usd());
    let err = try_with(&mut m.w.svm, |s| buy_at_ix(s, buyer.pubkey(), usd_acc, reading, 1, PRICE, 150), &buyer).unwrap_err();
    assert!(err.contains("RateRaised"), "{err}");
}

#[test]
fn the_replayer_is_paid_into_their_own_account_only() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    charge(&mut m, &buyer, purchase).unwrap();
    replay(&mut m, purchase, 2);
    let mut ix = settle_chargeback_ix(&mut m.w.svm, purchase);
    let someone = new_buyer(&mut m.w.svm, 0);
    let last = ix.accounts.len() - 2;
    ix.accounts[last].pubkey = ata(someone.pubkey(), usd());
    let err = try_ix(&mut m.w.svm, ix, &buyer).unwrap_err();
    assert!(err.contains("NotTheReader"), "{err}");
}

#[test]
fn seven_days_with_no_replay_pay_the_buyer() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    charge(&mut m, &buyer, purchase).unwrap();
    days_pass(&mut m.w.svm, 7);
    let err = settle(&mut m, purchase).unwrap_err();
    assert!(err.contains("NotLapsedYet"), "{err}");
    days_pass(&mut m.w.svm, 1);
    let before = usd_balance(&m.w.svm, buyer.pubkey());
    settle(&mut m, purchase).unwrap();
    assert_eq!(usd_balance(&m.w.svm, buyer.pubkey()), before + PRICE);
}

#[test]
fn settling_waits_for_a_replay_that_can_still_be_revealed() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    charge(&mut m, &buyer, purchase).unwrap();
    let c = chargeback(&m.w.svm, purchase);
    let slot = c.draw_slot + 40;
    at_slot(&mut m.w.svm, slot, &recent(slot, &[]));
    let reader = m.reader.insecure_clone();
    try_ix(&mut m.w.svm, record_draw_ix(purchase), &reader).unwrap();
    let q = sha256(&canonical_question(&[1u8; 16]));
    try_with(&mut m.w.svm, |s| commit_replay_ix(s, reader.pubkey(), purchase, SERVICE, q), &reader).unwrap();
    days_pass(&mut m.w.svm, 8);
    let err = settle(&mut m, purchase).unwrap_err();
    assert!(err.contains("ReplayStands"), "{err}");
}

#[test]
fn a_draw_never_recorded_counts_only_once_its_hour_is_over() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    charge(&mut m, &buyer, purchase).unwrap();
    let c = chargeback(&m.w.svm, purchase);
    let anyone = m.reader.insecure_clone();
    // Gone from SlotHashes after a few minutes, but its hour is not over.
    let gone = c.draw_slot + 2_000;
    at_slot(&mut m.w.svm, gone, &recent(gone, &[]));
    let err = try_with(&mut m.w.svm, |s| pass_draw_ix(s, purchase), &anyone).unwrap_err();
    assert!(err.contains("TooEarly"), "{err}");
    let late = c.draw_slot + 32 + READ_WINDOW_SLOTS + 1;
    at_slot(&mut m.w.svm, late, &recent(late, &[]));
    try_with(&mut m.w.svm, |s| pass_draw_ix(s, purchase), &anyone).unwrap();
}

#[test]
fn a_pass_must_show_the_seat_the_draw_gave() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    charge(&mut m, &buyer, purchase).unwrap();
    let c = chargeback(&m.w.svm, purchase);
    at_slot(&mut m.w.svm, c.draw_slot + 40, &recent(c.draw_slot + 40, &[]));
    try_ix(&mut m.w.svm, record_draw_ix(purchase), &buyer).unwrap();
    let entropy_slot = chargeback(&m.w.svm, purchase).entropy_slot;
    m.w.svm.warp_to_slot(entropy_slot + READ_WINDOW_SLOTS + 1);
    // Claim the quoter's seat was the one drawn, to have them marked declined.
    let mut ix = pass_draw_ix(&m.w.svm, purchase);
    ix.data = anchor_lang::InstructionData::data(&sasona::instruction::PassDraw { drawn_seat: 1 });
    ix.accounts.truncate(4);
    ix.accounts.push(AccountMeta::new_readonly(seat_address(1), false));
    let err = try_ix(&mut m.w.svm, ix, &buyer).unwrap_err();
    assert!(err.contains("NotDrawn") || err.contains("NotSkippable"), "{err}");
    // Or show one account too many.
    let mut ix = pass_draw_ix(&m.w.svm, purchase);
    ix.accounts.push(AccountMeta::new_readonly(seat_address(1), false));
    let err = try_ix(&mut m.w.svm, ix, &buyer).unwrap_err();
    assert!(err.contains("NotDrawn"), "{err}");
    try_with(&mut m.w.svm, |s| pass_draw_ix(s, purchase), &buyer).unwrap();
}

#[test]
fn a_purchase_closes_only_after_its_seven_days_and_not_once_charged_back() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    let pu: Purchase = read(&m.w.svm, purchase);
    let mut c: solana_clock::Clock = m.w.svm.get_sysvar();
    c.unix_timestamp = pu.made_time + 7 * 86_400;
    m.w.svm.set_sysvar(&c);
    let anyone = m.reader.insecure_clone();
    let err = try_with(&mut m.w.svm, |s| close_purchase_ix(s, purchase), &anyone).unwrap_err();
    assert!(err.contains("NotLapsedYet"), "at exactly seven days it can still be charged back: {err}");
    charge(&mut m, &buyer, purchase).unwrap();
    days_pass(&mut m.w.svm, 1);
    let err = try_with(&mut m.w.svm, |s| close_purchase_ix(s, purchase), &anyone).unwrap_err();
    assert!(err.contains("NotOpen"), "{err}");
}

#[test]
fn an_upheld_challenge_pays_the_debt_first_and_the_challenger_a_tenth_of_the_rest() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    charge(&mut m, &buyer, purchase).unwrap();
    replay(&mut m, purchase, 2);
    settle(&mut m, purchase).unwrap();
    let owed = chargeback(&m.w.svm, purchase).owed_coins;
    assert!(owed > 0);
    let c = challenger(&mut m.w.svm);
    let reading = m.reading;
    try_with(&mut m.w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap();
    days_pass(&mut m.w.svm, 8);
    let coins_before = coins(&m.w.svm, c.pubkey());
    let cover_before = cover(&m.w.svm).coins;
    try_with(&mut m.w.svm, |s| uphold_ix(s, reading), &c).unwrap();
    let reward = (MEMBER_STAKE - owed) / 10;
    assert_eq!(coins(&m.w.svm, c.pubkey()), coins_before + reward);
    assert_eq!(cover(&m.w.svm).coins, cover_before + MEMBER_STAKE - reward);
    // The stake already went to the cover, so repaying takes nothing more and
    // ends the debt.
    let r: sasona::Reading = read(&m.w.svm, m.reading);
    let now = m.w.svm.get_sysvar::<solana_clock::Clock>().unix_timestamp;
    days_pass(&mut m.w.svm, (r.reveal_time + CHALLENGE_WINDOW_SECONDS - now) / 86_400 + 1);
    let cover_before = cover(&m.w.svm).coins;
    try_with(&mut m.w.svm, |s| repay_cover_ix(s, purchase), &c).unwrap();
    assert_eq!(cover(&m.w.svm).coins, cover_before);
    assert_eq!((book(&m.w.svm, 1).owed_coins, chargeback(&m.w.svm, purchase).owed_coins), (0, 0));
}

#[test]
fn a_member_who_asked_to_leave_insures_nothing_more() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let quoter = m.quoter.insecure_clone();
    try_with(&mut m.w.svm, |s| ask_to_leave_ix(s, quoter.pubkey(), 1), &quoter).unwrap();
    let err = buy(&mut m, &buyer, 1, PRICE).unwrap_err();
    assert!(err.contains("NotActive"), "{err}");
}

#[test]
fn the_price_goes_only_to_the_address_the_reading_recorded() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let usd_acc = ata(buyer.pubkey(), usd());
    let mut ix = buy_ix(&m.w.svm, buyer.pubkey(), usd_acc, m.reading, 1, PRICE);
    let merchant = merchant_usd(&mut m.w.svm);
    let at = ix.accounts.iter().position(|a| a.pubkey == merchant).unwrap();
    // The buyer names themselves as the merchant, to be paid their own price.
    let own = new_buyer(&mut m.w.svm, 0);
    ix.accounts[at].pubkey = ata(own.pubkey(), usd());
    let err = try_ix(&mut m.w.svm, ix, &buyer).unwrap_err();
    assert!(err.contains("NotThePayTo"), "{err}");
}

fn set_last_chargeback(svm: &mut LiteSVM, purchase: Address, at: i64) {
    let pu: Purchase = read(svm, purchase);
    let address = pda(&[sasona::SERVICE_SEED, &pu.service]);
    let mut acc = svm.get_account(&address).unwrap();
    let mut terms: sasona::ServiceTerms = read(svm, address);
    terms.last_time = at;
    let mut data = Vec::new();
    anchor_lang::AccountSerialize::try_serialize(&terms, &mut data).unwrap();
    acc.data[..data.len()].copy_from_slice(&data);
    svm.set_account(address, acc).unwrap();
}

#[test]
fn a_paid_chargeback_also_starts_the_thirty_days() {
    let mut m = market();
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let p: Vec<Address> = (1..=3).map(|id| buy(&mut m, &buyer, id, 300_000).unwrap()).collect();
    charge(&mut m, &buyer, p[0]).unwrap();
    // Say the last chargeback on the service was 27 days ago.
    let now = m.w.svm.get_sysvar::<solana_clock::Clock>().unix_timestamp;
    set_last_chargeback(&mut m.w.svm, p[0], now - 27 * 86_400);
    days_pass(&mut m.w.svm, 2);
    charge(&mut m, &buyer, p[1]).unwrap();
    assert!(chargeback(&m.w.svm, p[1]).deposit > 0, "29 days after the last");
    // 31 days after the free one, but 2 after the paid one.
    days_pass(&mut m.w.svm, 2);
    charge(&mut m, &buyer, p[2]).unwrap();
    assert!(chargeback(&m.w.svm, p[2]).deposit > 0, "the paid one restarted the thirty days");
}

#[test]
fn a_seat_shown_as_passed_over_must_not_qualify() {
    // A third member, so that two seats qualify.
    let mut w = world_with_member();
    let quoter = w.depositor.insecure_clone();
    let reading = insured_reading(&mut w, SERVICE, 150, 1_000);
    let (reader, _) = new_member(&mut w.svm);
    let (third, _) = new_member(&mut w.svm);
    w.svm.warp_to_slot(1_100);
    let mut m = Market { w, quoter, reader, reading };
    let buyer = new_buyer(&mut m.w.svm, 10 * DOLLAR);
    let purchase = buy(&mut m, &buyer, 1, PRICE).unwrap();
    charge(&mut m, &buyer, purchase).unwrap();
    let c = chargeback(&m.w.svm, purchase);
    at_slot(&mut m.w.svm, c.draw_slot + 40, &recent(c.draw_slot + 40, &[]));
    try_ix(&mut m.w.svm, record_draw_ix(purchase), &buyer).unwrap();
    let drawn = replay_drawn_for(&m.w.svm, purchase).0.unwrap();
    let other = if drawn == 2 { 3 } else { 2 };
    let order = draw_order(&m.w.svm, purchase);
    let first_drawn = order.iter().position(|&k| k == drawn).unwrap();
    assert!(order[first_drawn..].contains(&other), "the draw gives the other seat later too");
    // The other member claims their seat, showing the drawn seat as passed over.
    let claimer = if other == 2 { m.reader.insecure_clone() } else { third };
    let q = sha256(&canonical_question(&[1u8; 16]));
    let err = try_with(&mut m.w.svm, |s| claim_replay_ix(s, purchase, other, SERVICE, q), &claimer).unwrap_err();
    assert!(err.contains("NotSkippable"), "{err}");
}
