//! Quotes: the member who read a service says what they would charge to
//! insure it. Run against the compiled program.

mod common;
use common::*;
use sasona::{canonical_question, Quote, CHALLENGE_WINDOW_SECONDS};

const SERVICE: &str = "https://sandbox.example.net/run/python";
const NONCE: [u8; 16] = [3u8; 16];

/// Membership 1 reads SERVICE and records `verdict`, unrevealed if `reveal`
/// is false.
fn reading(w: &mut World, verdict: u8, reveal: bool) -> Address {
    let d = w.depositor.insecure_clone();
    let seed = [1u8; 32];
    w.svm.warp_to_slot(1_000);
    try_ix(&mut w.svm, open_round_ix(d.pubkey(), [1u8; 32], 10, 2, sha256(&seed)), &d).unwrap();
    at_slot(&mut w.svm, 1_040, &recent(1_040, &[]));
    let round = round_address([1u8; 32]);
    try_ix(&mut w.svm, reveal_ix(round, d.pubkey(), seed), &d).unwrap();
    w.svm.warp_to_slot(1_100);
    let q = sha256(&canonical_question(&NONCE));
    try_with(&mut w.svm, |s| commit_reading_ix(s, d.pubkey(), round, SERVICE, q), &d).unwrap();
    let reading = reading_address(round, SERVICE);
    if reveal {
        w.svm.warp_to_slot(1_101);
        try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), reading, NONCE, [9u8; 32], verdict), &d).unwrap();
    }
    reading
}

fn quote(w: &mut World, who: &Keypair, reading: Address, rate: u16) -> Result<(), String> {
    try_with(&mut w.svm, |s| set_quote_ix(s, who.pubkey(), reading, rate), who)
}

fn seconds_pass(svm: &mut LiteSVM, seconds: i64) {
    let mut c: solana_clock::Clock = svm.get_sysvar();
    c.unix_timestamp += seconds;
    svm.set_sysvar(&c);
}

#[test]
fn the_reader_quotes_changes_and_withdraws() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let r = reading(&mut w, 1, true);
    quote(&mut w, &d, r, 120).unwrap();
    let q: Quote = read(&w.svm, quote_address(r));
    let now = w.svm.get_sysvar::<solana_clock::Clock>().unix_timestamp;
    assert_eq!((addr(q.reading), q.member, q.rate, q.set_time), (r, 1, 120, now));

    seconds_pass(&mut w.svm, 60);
    quote(&mut w, &d, r, 450).unwrap();
    let q: Quote = read(&w.svm, quote_address(r));
    assert_eq!((q.rate, q.set_time), (450, now + 60));
    quote(&mut w, &d, r, 0).unwrap();
    let q: Quote = read(&w.svm, quote_address(r));
    assert_eq!(q.rate, 0);
    quote(&mut w, &d, r, 10_000).unwrap();
}

#[test]
fn only_the_reader_quotes() {
    let mut w = world_with_member();
    let r = reading(&mut w, 1, true);
    let (stranger, _) = newcomer(&mut w.svm, 0);
    let err = quote(&mut w, &stranger, r, 120).unwrap_err();
    assert!(err.contains("NotTheReader"), "{err}");
}

#[test]
fn a_rate_is_at_most_ten_thousand_basis_points() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let r = reading(&mut w, 1, true);
    let err = quote(&mut w, &d, r, 10_001).unwrap_err();
    assert!(err.contains("BadRate"), "{err}");
}

#[test]
fn a_service_that_did_not_deliver_is_not_insured() {
    for verdict in [2u8, 3] {
        let mut w = world_with_member();
        let d = w.depositor.insecure_clone();
        let r = reading(&mut w, verdict, true);
        let err = quote(&mut w, &d, r, 120).unwrap_err();
        assert!(err.contains("NotDelivered"), "verdict {verdict}: {err}");
    }
}

#[test]
fn an_unrevealed_reading_is_not_quoted() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let r = reading(&mut w, 1, false);
    let err = quote(&mut w, &d, r, 120).unwrap_err();
    assert!(err.contains("ReadingNotOpen"), "{err}");
}

#[test]
fn a_reading_is_quoted_for_thirty_days_and_withdrawn_at_any_time() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let r = reading(&mut w, 1, true);
    seconds_pass(&mut w.svm, CHALLENGE_WINDOW_SECONDS);
    quote(&mut w, &d, r, 120).unwrap();
    seconds_pass(&mut w.svm, 1);
    let err = quote(&mut w, &d, r, 130).unwrap_err();
    assert!(err.contains("WindowClosed"), "{err}");
    quote(&mut w, &d, r, 0).unwrap();
}

#[test]
fn a_member_who_asked_to_leave_insures_nothing_new() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let r = reading(&mut w, 1, true);
    quote(&mut w, &d, r, 120).unwrap();
    try_with(&mut w.svm, |s| ask_to_leave_ix(s, d.pubkey(), 1), &d).unwrap();
    let err = quote(&mut w, &d, r, 100).unwrap_err();
    assert!(err.contains("NotActive"), "{err}");
    quote(&mut w, &d, r, 0).unwrap();
}

#[test]
fn a_reading_upheld_false_is_not_quoted() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let r = reading(&mut w, 1, true);
    let (c, usd) = newcomer(&mut w.svm, 10);
    try_deposit(&mut w.svm, &c, usd, 10 * DOLLAR).unwrap();
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), r), &c).unwrap();
    days_pass(&mut w.svm, 8);
    try_with(&mut w.svm, |s| uphold_ix(s, r), &c).unwrap();
    let err = quote(&mut w, &d, r, 120).unwrap_err();
    assert!(err.contains("ReadingNotOpen") || err.contains("NotActive"), "{err}");
}
