//! Second readings: a reading in a re-read round that names the first one it
//! re-tests, and what the pair settles. Run against the compiled program.

mod common;
use common::*;
use sasona::{canonical_question, pair_outcome, Pair, PAIR_AGREED_FAILS, PAIR_FALSE_OR_DECAYED, PAIR_WORKS_NOW};

const SERVICE: &str = "https://sandbox.example.net/run/python";

fn nonce(n: u8) -> [u8; 16] {
    [n; 16]
}

/// A drawn round opened by `who`, with its own list fingerprint `fp`.
fn drawn_round(w: &mut World, who: &Keypair, fp: u8, slot: u64) -> Address {
    let seed = [fp; 32];
    w.svm.warp_to_slot(slot);
    try_ix(&mut w.svm, open_round_ix(who.pubkey(), [fp; 32], 10, 2, sha256(&seed)), who).unwrap();
    at_slot(&mut w.svm, slot + 40, &recent(slot + 40, &[]));
    try_ix(&mut w.svm, reveal_ix(round_address([fp; 32]), who.pubkey(), seed), who).unwrap();
    round_address([fp; 32])
}

/// A first reading of SERVICE by the depositor with `verdict`, revealed by
/// slot 1,050, and a re-read round opened by someone else, drawn by 1,140.
fn first_and_rereader(verdict: u8) -> (World, Address, Keypair, Address) {
    let mut w = world();
    let d = w.depositor.insecure_clone();
    let round = drawn_round(&mut w, &d, 1, 1_000);
    try_ix(&mut w.svm, commit_reading_ix(d.pubkey(), round, SERVICE, sha256(&canonical_question(&nonce(1)))), &d).unwrap();
    let first = reading_address(round, SERVICE);
    w.svm.warp_to_slot(1_050);
    try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), first, nonce(1), [1u8; 32], verdict), &d).unwrap();

    let (other, _) = newcomer(&mut w.svm, 0);
    let reread = drawn_round(&mut w, &other, 2, 1_100);
    (w, first, other, reread)
}

fn second(w: &mut World, reader: &Keypair, reread: Address, first: Address, verdict: u8) -> Address {
    try_ix(&mut w.svm, commit_second_ix(reader.pubkey(), reread, SERVICE, sha256(&canonical_question(&nonce(2))), first), reader).unwrap();
    let s = reading_address(reread, SERVICE);
    let slot = w.svm.get_sysvar::<solana_clock::Clock>().slot;
    w.svm.warp_to_slot(slot + 1);
    try_ix(&mut w.svm, reveal_reading_ix(reader.pubkey(), s, nonce(2), [2u8; 32], verdict), reader).unwrap();
    s
}

#[test]
fn every_pair_settles_as_the_protocol_says() {
    // vectors/pair.json in sasona-protocol 0.4.0, all nine.
    for (first, second, outcome) in [
        (1, 1, PAIR_WORKS_NOW), (1, 2, PAIR_FALSE_OR_DECAYED), (1, 3, PAIR_FALSE_OR_DECAYED),
        (2, 1, PAIR_WORKS_NOW), (2, 2, PAIR_AGREED_FAILS), (2, 3, PAIR_AGREED_FAILS),
        (3, 1, PAIR_WORKS_NOW), (3, 2, PAIR_AGREED_FAILS), (3, 3, PAIR_AGREED_FAILS),
    ] {
        assert_eq!(pair_outcome(first, second), outcome, "{first} then {second}");
    }
}

#[test]
fn a_second_reading_is_recorded_and_settled() {
    for (v1, v2, want) in [(1u8, 1u8, PAIR_WORKS_NOW), (1, 2, PAIR_FALSE_OR_DECAYED), (2, 2, PAIR_AGREED_FAILS)] {
        let (mut w, first, other, reread) = first_and_rereader(v1);
        let s = second(&mut w, &other, reread, first, v2);
        let pair = pda(&[sasona::PAIR_SEED, s.as_ref()]);
        let p: Pair = read(&w.svm, pair);
        assert_eq!((addr(p.first), addr(p.second), p.outcome), (first, s, 0));

        let (anyone, _) = newcomer(&mut w.svm, 0);
        try_ix(&mut w.svm, settle_pair_ix(first, s), &anyone).unwrap();
        let p: Pair = read(&w.svm, pair);
        assert_eq!(p.outcome, want, "{v1} then {v2}");

        let err = try_ix(&mut w.svm, settle_pair_ix(first, s), &anyone).unwrap_err();
        assert!(err.contains("AlreadySettled"), "{err}");
    }
}

#[test]
fn the_first_reader_cannot_check_themselves() {
    let (mut w, first, _, _) = first_and_rereader(1);
    let d = w.depositor.insecure_clone();
    let own = drawn_round(&mut w, &d, 3, 1_200);
    let err = try_ix(&mut w.svm, commit_second_ix(d.pubkey(), own, SERVICE, [0u8; 32], first), &d).unwrap_err();
    assert!(err.contains("SameReader"), "{err}");
}

#[test]
fn a_second_reading_is_of_the_same_service() {
    let (mut w, first, other, reread) = first_and_rereader(1);
    let err = try_ix(&mut w.svm, commit_second_ix(other.pubkey(), reread, "https://other.example.org/x", [0u8; 32], first), &other)
        .unwrap_err();
    assert!(err.contains("NotTheSameService"), "{err}");
}

#[test]
fn a_second_reading_cannot_be_taken_in_the_first_ones_round() {
    // In the first's round, the address for this service's reading is the
    // first reading itself, so there is nowhere to put a second.
    let (mut w, first, _, _) = first_and_rereader(1);
    let d = w.depositor.insecure_clone();
    let err = try_ix(&mut w.svm, commit_second_ix(d.pubkey(), round_address([1u8; 32]), SERVICE, [0u8; 32], first), &d)
        .unwrap_err();
    assert!(err.contains("already in use"), "{err}");
}

#[test]
fn a_second_reading_cannot_start_in_the_slot_the_first_was_revealed() {
    let mut w = world();
    let d = w.depositor.insecure_clone();
    let (other, _) = newcomer(&mut w.svm, 0);
    let round = drawn_round(&mut w, &d, 1, 1_000);
    let reread = drawn_round(&mut w, &other, 2, 1_100);
    try_ix(&mut w.svm, commit_reading_ix(d.pubkey(), round, SERVICE, sha256(&canonical_question(&nonce(1)))), &d).unwrap();
    let first = reading_address(round, SERVICE);
    w.svm.warp_to_slot(1_200);
    try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), first, nonce(1), [1u8; 32], 1), &d).unwrap();
    let err = try_ix(&mut w.svm, commit_second_ix(other.pubkey(), reread, SERVICE, [0u8; 32], first), &other).unwrap_err();
    assert!(err.contains("TooEarly"), "{err}");
    w.svm.warp_to_slot(1_201);
    try_ix(&mut w.svm, commit_second_ix(other.pubkey(), reread, SERVICE, [0u8; 32], first), &other).unwrap();
}

#[test]
fn only_a_revealed_reading_can_be_re_tested() {
    let mut w = world();
    let d = w.depositor.insecure_clone();
    let round = drawn_round(&mut w, &d, 1, 1_000);
    try_ix(&mut w.svm, commit_reading_ix(d.pubkey(), round, SERVICE, [0u8; 32]), &d).unwrap();
    let first = reading_address(round, SERVICE);
    let (other, _) = newcomer(&mut w.svm, 0);
    let reread = drawn_round(&mut w, &other, 2, 1_100);
    let err = try_ix(&mut w.svm, commit_second_ix(other.pubkey(), reread, SERVICE, [0u8; 32], first), &other).unwrap_err();
    assert!(err.contains("FirstNotRevealed"), "{err}");
}

#[test]
fn a_pair_settles_only_once_its_second_is_revealed() {
    let (mut w, first, other, reread) = first_and_rereader(1);
    try_ix(&mut w.svm, commit_second_ix(other.pubkey(), reread, SERVICE, sha256(&canonical_question(&nonce(2))), first), &other).unwrap();
    let s = reading_address(reread, SERVICE);
    let (anyone, _) = newcomer(&mut w.svm, 0);
    let err = try_ix(&mut w.svm, settle_pair_ix(first, s), &anyone).unwrap_err();
    assert!(err.contains("ReadingNotOpen"), "{err}");
}

#[test]
fn a_pair_settles_against_its_own_first_reading_only() {
    let (mut w, first, other, reread) = first_and_rereader(1);
    let s = second(&mut w, &other, reread, first, 2);
    let (anyone, _) = newcomer(&mut w.svm, 0);
    // Passing the second reading as its own first would make it agree with itself.
    let err = try_ix(&mut w.svm, settle_pair_ix(s, s), &anyone).unwrap_err();
    assert!(err.contains("ConstraintHasOne"), "{err}");
}
