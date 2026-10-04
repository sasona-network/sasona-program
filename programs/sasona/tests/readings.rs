//! Readings: a question committed before a drawn service is called, and
//! revealed by its nonce with the reply's hash and the verdict after. Run
//! against the compiled program.

mod common;
use common::*;
use sasona::{canonical_question, Reading, READING_COMMITTED, READING_LAPSED, READING_REVEALED, REVEAL_WINDOW_SLOTS};

const SERVICE: &str = "https://sandbox.example.net/run/python";

// The first question in sasona-protocol's vectors/question.json, 0.3.0.
const VECTOR_NONCE: [u8; 16] = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
const VECTOR_QUESTION_HASH: &str = "0a3c847b1edb39cd948bbc3d4d5629b62dc407e295d9c26235e715fd2077507c";

fn hex32(s: &str) -> [u8; 32] {
    let v: Vec<u8> = (0..32).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect();
    v.try_into().unwrap()
}

/// A world with a round opened by the depositor and drawn, at slot 1,100.
fn drawn() -> (World, Address) {
    let mut w = world();
    let d = w.depositor.insecure_clone();
    let seed = [42u8; 32];
    let fp = [77u8; 32];
    at_slot(&mut w.svm, 1_000, &recent(1_000, &[]));
    try_ix(&mut w.svm, open_round_ix(d.pubkey(), fp, 10, 2, sha256(&seed)), &d).unwrap();
    at_slot(&mut w.svm, 1_100, &recent(1_100, &[]));
    try_ix(&mut w.svm, reveal_ix(round_address(fp), d.pubkey(), seed), &d).unwrap();
    (w, round_address(fp))
}

fn commit(w: &mut World, round: Address, endpoint: &str, nonce: [u8; 16]) -> Address {
    let d = w.depositor.insecure_clone();
    let q = sha256(&canonical_question(&nonce));
    try_ix(&mut w.svm, commit_reading_ix(d.pubkey(), round, endpoint, q), &d).unwrap();
    reading_address(round, endpoint)
}

#[test]
fn the_program_builds_the_protocols_question() {
    // Every question in sasona-protocol's vectors/question.json, 0.3.0.
    for (nonce, hash) in [
        ("0123456789abcdef0123456789abcdef", VECTOR_QUESTION_HASH),
        ("ffffffffffffffffffffffffffffffff", "a4ec2e4c1abc7d36a394d105d0f6df50c06138b4f9861a4981bec8cda72f9449"),
        ("9a3e5c0d71b24f68aa0b1c2d3e4f5061", "d7128e219c21fa19f335e690a70f062ede149a6f385a4ec554afffdc94952a9c"),
    ] {
        let raw: [u8; 16] = (0..16).map(|i| u8::from_str_radix(&nonce[2 * i..2 * i + 2], 16).unwrap()).collect::<Vec<_>>().try_into().unwrap();
        assert_eq!(sha256(&canonical_question(&raw)), hex32(hash), "the program and the specification disagree for {nonce}");
    }
}

fn nonce_owner(w: &World, nonce: [u8; 16]) -> Address {
    let n: sasona::UsedNonce = read(&w.svm, pda(&[sasona::NONCE_SEED, &nonce]));
    addr(n.reading)
}

#[test]
fn a_service_copying_the_nonce_cannot_block_the_honest_reading() {
    // The attack the review found: the service sees the nonce when it is
    // called, opens a round of its own, commits the same question and
    // reveals it first, so the honest reading could never be revealed.
    let (mut w, round) = drawn();
    let honest = commit(&mut w, round, SERVICE, VECTOR_NONCE); // slot 1,100

    let (service, _) = newcomer(&mut w.svm, 0);
    let fp = [91u8; 32];
    w.svm.warp_to_slot(1_101);
    try_ix(&mut w.svm, open_round_ix(service.pubkey(), fp, 5, 1, sha256(&[2u8; 32])), &service).unwrap();
    at_slot(&mut w.svm, 1_140, &recent(1_140, &[]));
    try_ix(&mut w.svm, reveal_ix(round_address(fp), service.pubkey(), [2u8; 32]), &service).unwrap();
    let q = sha256(&canonical_question(&VECTOR_NONCE));
    try_ix(&mut w.svm, commit_reading_ix(service.pubkey(), round_address(fp), "https://anything.example/x", q), &service).unwrap();
    let copy = reading_address(round_address(fp), "https://anything.example/x");
    w.svm.warp_to_slot(1_141);
    try_ix(&mut w.svm, reveal_reading_ix(service.pubkey(), copy, VECTOR_NONCE, [3u8; 32], 2), &service).unwrap();
    assert_eq!(nonce_owner(&w, VECTOR_NONCE), copy);

    // The honest reading was committed first, so it takes the nonce back.
    w.svm.warp_to_slot(1_142);
    let d = w.depositor.insecure_clone();
    try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), honest, VECTOR_NONCE, [4u8; 32], 2), &d).unwrap();
    assert_eq!(nonce_owner(&w, VECTOR_NONCE), honest);
    let r: Reading = read(&w.svm, honest);
    assert_eq!(r.state, READING_REVEALED);
}

#[test]
fn reaching_the_end_of_the_window_still_reveals() {
    let (mut w, round) = drawn();
    let reading = commit(&mut w, round, SERVICE, VECTOR_NONCE);
    w.svm.warp_to_slot(1_100 + REVEAL_WINDOW_SLOTS);
    let d = w.depositor.insecure_clone();
    try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), reading, VECTOR_NONCE, [3u8; 32], 1), &d).unwrap();
    let (anyone, _) = newcomer(&mut w.svm, 0);
    w.svm.warp_to_slot(1_100 + REVEAL_WINDOW_SLOTS + 5);
    let err = try_ix(&mut w.svm, lapsed_ix(reading), &anyone).unwrap_err();
    assert!(err.contains("ReadingNotOpen"), "a revealed reading cannot lapse: {err}");
}

#[test]
fn only_a_reading_can_be_marked_lapsed() {
    let (mut w, round) = drawn();
    let (anyone, _) = newcomer(&mut w.svm, 0);
    let err = try_ix(&mut w.svm, lapsed_ix(round), &anyone).unwrap_err();
    assert!(err.contains("AccountDiscriminatorMismatch"), "{err}");
}

#[test]
fn a_reading_is_committed_then_revealed() {
    let (mut w, round) = drawn();
    let reading = commit(&mut w, round, SERVICE, VECTOR_NONCE);
    let r: Reading = read(&w.svm, reading);
    assert_eq!((addr(r.round), r.endpoint.as_str(), addr(r.reader)), (round, SERVICE, w.depositor.pubkey()));
    assert_eq!((r.question_hash, r.commit_slot, r.state), (hex32(VECTOR_QUESTION_HASH), 1_100, READING_COMMITTED));

    w.svm.warp_to_slot(1_101);
    let d = w.depositor.insecure_clone();
    try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), reading, VECTOR_NONCE, [3u8; 32], 1), &d).unwrap();
    let r: Reading = read(&w.svm, reading);
    assert_eq!((r.state, r.verdict, r.reply_hash, r.reveal_slot), (READING_REVEALED, 1, [3u8; 32], 1_101));
}

#[test]
fn only_the_fair_question_for_the_nonce_can_be_revealed() {
    // Committing to anything else, even the same object written differently,
    // can never be revealed.
    let (mut w, round) = drawn();
    let d = w.depositor.insecure_clone();
    let mut unfair = canonical_question(&VECTOR_NONCE);
    unfair.push(b' ');
    try_ix(&mut w.svm, commit_reading_ix(d.pubkey(), round, SERVICE, sha256(&unfair)), &d).unwrap();
    w.svm.warp_to_slot(1_101);
    let reading = reading_address(round, SERVICE);
    let err = try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), reading, VECTOR_NONCE, [3u8; 32], 1), &d).unwrap_err();
    assert!(err.contains("WrongQuestion"), "{err}");
}

#[test]
fn a_different_nonce_is_refused() {
    let (mut w, round) = drawn();
    let reading = commit(&mut w, round, SERVICE, VECTOR_NONCE);
    w.svm.warp_to_slot(1_101);
    let d = w.depositor.insecure_clone();
    let err = try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), reading, [9u8; 16], [3u8; 32], 1), &d).unwrap_err();
    assert!(err.contains("WrongQuestion"), "{err}");
}

#[test]
fn a_nonce_is_never_used_twice() {
    let (mut w, round) = drawn();
    let first = commit(&mut w, round, SERVICE, VECTOR_NONCE);
    let same_slot = commit(&mut w, round, "https://other.example.org/run", VECTOR_NONCE);
    w.svm.warp_to_slot(1_150);
    let later = commit(&mut w, round, "https://third.example.org/run", VECTOR_NONCE);
    w.svm.warp_to_slot(1_151);
    let d = w.depositor.insecure_clone();
    try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), first, VECTOR_NONCE, [3u8; 32], 1), &d).unwrap();
    for (name, reading) in [("committed in the same slot", same_slot), ("committed later", later)] {
        let err = try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), reading, VECTOR_NONCE, [3u8; 32], 1), &d).unwrap_err();
        assert!(err.contains("NonceTaken"), "{name}: {err}");
    }
    assert_eq!(nonce_owner(&w, VECTOR_NONCE), first);
}

#[test]
fn revealing_in_the_same_slot_as_the_commitment_is_refused() {
    let (mut w, round) = drawn();
    let reading = commit(&mut w, round, SERVICE, VECTOR_NONCE);
    let d = w.depositor.insecure_clone();
    let err = try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), reading, VECTOR_NONCE, [3u8; 32], 1), &d).unwrap_err();
    assert!(err.contains("TooEarly"), "{err}");
}

#[test]
fn only_a_known_verdict_is_accepted() {
    let (mut w, round) = drawn();
    let reading = commit(&mut w, round, SERVICE, VECTOR_NONCE);
    w.svm.warp_to_slot(1_101);
    let d = w.depositor.insecure_clone();
    for bad in [0u8, 4, 255] {
        let err = try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), reading, VECTOR_NONCE, [3u8; 32], bad), &d).unwrap_err();
        assert!(err.contains("BadVerdict"), "{bad}: {err}");
    }
}

#[test]
fn a_reading_is_revealed_once_and_only_by_its_reader() {
    let (mut w, round) = drawn();
    let reading = commit(&mut w, round, SERVICE, VECTOR_NONCE);
    w.svm.warp_to_slot(1_101);
    let (stranger, _) = newcomer(&mut w.svm, 0);
    let err = try_ix(&mut w.svm, reveal_reading_ix(stranger.pubkey(), reading, VECTOR_NONCE, [3u8; 32], 1), &stranger).unwrap_err();
    assert!(err.contains("NotTheReader"), "{err}");
    let d = w.depositor.insecure_clone();
    try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), reading, VECTOR_NONCE, [3u8; 32], 1), &d).unwrap();
    let err = try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), reading, [5u8; 16], [4u8; 32], 2), &d).unwrap_err();
    assert!(err.contains("ReadingNotOpen"), "{err}");
}

#[test]
fn a_reading_not_revealed_in_time_lapses() {
    let (mut w, round) = drawn();
    let reading = commit(&mut w, round, SERVICE, VECTOR_NONCE);
    let (anyone, _) = newcomer(&mut w.svm, 0);
    w.svm.warp_to_slot(1_100 + REVEAL_WINDOW_SLOTS);
    let err = try_ix(&mut w.svm, lapsed_ix(reading), &anyone).unwrap_err();
    assert!(err.contains("NotLapsedYet"), "{err}");

    w.svm.warp_to_slot(1_100 + REVEAL_WINDOW_SLOTS + 1);
    let d = w.depositor.insecure_clone();
    let err = try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), reading, VECTOR_NONCE, [3u8; 32], 1), &d).unwrap_err();
    assert!(err.contains("TooLate"), "{err}");
    try_ix(&mut w.svm, lapsed_ix(reading), &anyone).unwrap();
    let r: Reading = read(&w.svm, reading);
    assert_eq!(r.state, READING_LAPSED);
}

#[test]
fn one_reading_per_service_per_round() {
    let (mut w, round) = drawn();
    commit(&mut w, round, SERVICE, VECTOR_NONCE);
    let d = w.depositor.insecure_clone();
    let err = try_ix(&mut w.svm, commit_reading_ix(d.pubkey(), round, SERVICE, [1u8; 32]), &d).unwrap_err();
    assert!(err.contains("already in use"), "{err}");
}

#[test]
fn only_the_rounds_opener_reads_for_now() {
    let (mut w, round) = drawn();
    let (stranger, _) = newcomer(&mut w.svm, 0);
    let q = sha256(&canonical_question(&VECTOR_NONCE));
    let err = try_ix(&mut w.svm, commit_reading_ix(stranger.pubkey(), round, SERVICE, q), &stranger).unwrap_err();
    assert!(err.contains("NotTheReader"), "{err}");
}

#[test]
fn a_round_not_yet_drawn_cannot_be_read() {
    let mut w = world();
    let d = w.depositor.insecure_clone();
    at_slot(&mut w.svm, 1_000, &recent(1_000, &[]));
    try_ix(&mut w.svm, open_round_ix(d.pubkey(), [78u8; 32], 10, 2, sha256(&[1u8; 32])), &d).unwrap();
    let q = sha256(&canonical_question(&VECTOR_NONCE));
    let err = try_ix(&mut w.svm, commit_reading_ix(d.pubkey(), round_address([78u8; 32]), SERVICE, q), &d).unwrap_err();
    assert!(err.contains("RoundNotDrawn"), "{err}");
}

#[test]
fn the_service_must_be_a_clean_url_matching_its_hash() {
    let (mut w, round) = drawn();
    let d = w.depositor.insecure_clone();
    let q = sha256(&canonical_question(&VECTOR_NONCE));
    let long = format!("https://x.example/{}", "a".repeat(300));
    for bad in ["", "https://x.example/a b", long.as_str()] {
        let err = try_ix(&mut w.svm, commit_reading_ix(d.pubkey(), round, bad, q), &d).unwrap_err();
        assert!(err.contains("BadEndpoint"), "{bad:?}: {err}");
    }
    // The hash that places the reading must be the hash of the service named.
    let other = "https://other.example/x";
    let mut ix = commit_reading_ix(d.pubkey(), round, SERVICE, q);
    ix.accounts[2].pubkey = reading_address(round, other);
    ix.data = anchor_lang::InstructionData::data(&sasona::instruction::CommitReading {
        endpoint_hash: sha256(other.as_bytes()),
        endpoint: SERVICE.to_string(),
        question_hash: q,
    });
    let err = try_ix(&mut w.svm, ix, &d).unwrap_err();
    assert!(err.contains("BadEndpoint"), "{err}");
}
