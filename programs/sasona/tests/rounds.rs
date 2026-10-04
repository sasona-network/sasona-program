//! Rounds: committing to a draw, revealing it against a slot hash, and
//! marking one whose seed was withheld. Run against the compiled program.

mod common;
use common::*;
use sasona::{Round, ENTROPY_DELAY_SLOTS, MAX_ROUND_CANDIDATES, ROUND_BOND_LAMPORTS, ROUND_DRAWN, ROUND_WITHHELD};

fn hex32(s: &str) -> [u8; 32] {
    let v: Vec<u8> = (0..32).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect();
    v.try_into().unwrap()
}

// The first case in sasona-protocol's vectors/draw.json, version 0.1.0.
const VECTOR_SEED: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const VECTOR_ENTROPY: &str = "9764b6956cce2d60d31c9a1fc0ada705175c059bf34678c347ebacd5b6237078";
const VECTOR_SEED_HASH: &str = "630dcd2966c4336691125448bbb25b4ff412a49c732db2c8abc1b8581bd710dd";
const VECTOR_FINAL_SEED: &str = "01fb970c24eb651b1de93ee0ddf35fe0688e434294d4cd3059ed43c9d92bc69c";

const COMMIT: u64 = 1_000;

/// A list fingerprint, different for each test that needs its own round.
fn list(n: u8) -> [u8; 32] {
    [n; 32]
}

fn opened_round(w: &mut World, n: u8, count: u16) -> Address {
    at_slot(&mut w.svm, COMMIT, &recent(COMMIT, &[]));
    let d = w.depositor.insecure_clone();
    let ix = open_round_ix(d.pubkey(), list(n), 40, count, hex32(VECTOR_SEED_HASH));
    try_ix(&mut w.svm, ix, &d).unwrap();
    round_address(list(n))
}

fn lamports(svm: &LiteSVM, a: Address) -> u64 {
    svm.get_account(&a).map(|x| x.lamports).unwrap_or(0)
}

#[test]
fn a_round_records_its_commitment_and_holds_a_bond() {
    let mut w = world();
    let before = lamports(&w.svm, w.depositor.pubkey());
    let round = opened_round(&mut w, 7, 3);
    let r: Round = read(&w.svm, round);
    assert_eq!(addr(r.opener), w.depositor.pubkey());
    assert_eq!((r.pool_size, r.count, r.commit_slot, r.state), (40, 3, COMMIT, 0));
    assert_eq!(r.pool_fingerprint, list(7));
    assert_eq!(r.seed_hash, hex32(VECTOR_SEED_HASH));
    assert!(lamports(&w.svm, round) >= ROUND_BOND_LAMPORTS);
    assert!(before - lamports(&w.svm, w.depositor.pubkey()) >= ROUND_BOND_LAMPORTS);
}

#[test]
fn the_revealed_round_matches_the_protocol_vector() {
    // The program and sasona-protocol must agree on the final seed.
    let mut w = world();
    let round = opened_round(&mut w, 1, 3);
    let target = COMMIT + ENTROPY_DELAY_SLOTS;
    let mut entries = recent(COMMIT + 100, &[]);
    for e in entries.iter_mut() {
        if e.0 == target {
            e.1 = hex32(VECTOR_ENTROPY);
        }
    }
    at_slot(&mut w.svm, COMMIT + 100, &entries);

    let opener = w.depositor.pubkey();
    let bond_back_to = lamports(&w.svm, opener);
    let (anyone, _) = newcomer(&mut w.svm, 0);
    try_ix(&mut w.svm, reveal_ix(round, opener, hex32(VECTOR_SEED)), &anyone).unwrap();

    let r: Round = read(&w.svm, round);
    assert_eq!(r.state, ROUND_DRAWN);
    assert_eq!(r.entropy_slot, target);
    assert_eq!(r.entropy, hex32(VECTOR_ENTROPY));
    assert_eq!(r.seed, hex32(VECTOR_SEED));
    assert_eq!(r.final_seed, hex32(VECTOR_FINAL_SEED), "the program and the specification disagree");
    assert_eq!(lamports(&w.svm, opener), bond_back_to + ROUND_BOND_LAMPORTS, "the bond went back to the opener");
}

#[test]
fn a_skipped_target_slot_uses_the_next_one() {
    let mut w = world();
    let round = opened_round(&mut w, 2, 3);
    let target = COMMIT + ENTROPY_DELAY_SLOTS;
    at_slot(&mut w.svm, COMMIT + 100, &recent(COMMIT + 100, &[target, target + 1]));
    let (anyone, _) = newcomer(&mut w.svm, 0);
    try_ix(&mut w.svm, reveal_ix(round, w.depositor.pubkey(), hex32(VECTOR_SEED)), &anyone).unwrap();
    let r: Round = read(&w.svm, round);
    assert_eq!(r.entropy_slot, target + 2);
    assert_eq!(r.entropy, slot_hash(target + 2));
}

#[test]
fn revealing_before_the_target_slot_exists_is_refused() {
    let mut w = world();
    let round = opened_round(&mut w, 3, 3);
    let target = COMMIT + ENTROPY_DELAY_SLOTS;
    at_slot(&mut w.svm, target, &recent(target, &[]));
    let (anyone, _) = newcomer(&mut w.svm, 0);
    let err = try_ix(&mut w.svm, reveal_ix(round, w.depositor.pubkey(), hex32(VECTOR_SEED)), &anyone).unwrap_err();
    assert!(err.contains("TooEarly"), "{err}");
}

#[test]
fn the_wrong_seed_is_refused() {
    let mut w = world();
    let round = opened_round(&mut w, 4, 3);
    at_slot(&mut w.svm, COMMIT + 100, &recent(COMMIT + 100, &[]));
    let (anyone, _) = newcomer(&mut w.svm, 0);
    let err = try_ix(&mut w.svm, reveal_ix(round, w.depositor.pubkey(), [1u8; 32]), &anyone).unwrap_err();
    assert!(err.contains("WrongSeed"), "{err}");
}

#[test]
fn a_round_is_revealed_once() {
    let mut w = world();
    let round = opened_round(&mut w, 5, 3);
    at_slot(&mut w.svm, COMMIT + 100, &recent(COMMIT + 100, &[]));
    let (anyone, _) = newcomer(&mut w.svm, 0);
    let opener = w.depositor.pubkey();
    try_ix(&mut w.svm, reveal_ix(round, opener, hex32(VECTOR_SEED)), &anyone).unwrap();
    let err = try_ix(&mut w.svm, reveal_ix(round, opener, hex32(VECTOR_SEED)), &anyone).unwrap_err();
    assert!(err.contains("RoundNotOpen"), "{err}");
}

#[test]
fn the_bond_only_goes_to_the_opener() {
    let mut w = world();
    let round = opened_round(&mut w, 6, 3);
    at_slot(&mut w.svm, COMMIT + 100, &recent(COMMIT + 100, &[]));
    let (thief, _) = newcomer(&mut w.svm, 0);
    let err = try_ix(&mut w.svm, reveal_ix(round, thief.pubkey(), hex32(VECTOR_SEED)), &thief).unwrap_err();
    assert!(err.contains("ConstraintAddress"), "{err}");
}

#[test]
fn a_seed_kept_too_long_is_marked_withheld_and_the_bond_is_lost() {
    let mut w = world();
    let round = opened_round(&mut w, 8, 3);
    let target = COMMIT + ENTROPY_DELAY_SLOTS;
    let (anyone, _) = newcomer(&mut w.svm, 0);

    // While the target slot is still in SlotHashes it cannot be marked.
    at_slot(&mut w.svm, target + 400, &recent(target + 400, &[]));
    let err = try_ix(&mut w.svm, withheld_ix(round), &anyone).unwrap_err();
    assert!(err.contains("NotWithheldYet"), "{err}");

    // Once it has left, the seed can no longer be revealed, only marked.
    at_slot(&mut w.svm, target + 600, &recent(target + 600, &[]));
    let err = try_ix(&mut w.svm, reveal_ix(round, w.depositor.pubkey(), hex32(VECTOR_SEED)), &anyone).unwrap_err();
    assert!(err.contains("TooLate"), "{err}");
    let held = lamports(&w.svm, round);
    try_ix(&mut w.svm, withheld_ix(round), &anyone).unwrap();
    let r: Round = read(&w.svm, round);
    assert_eq!(r.state, ROUND_WITHHELD);
    assert_eq!(lamports(&w.svm, round), held, "the bond stays locked in the round");
    assert!(held >= ROUND_BOND_LAMPORTS);
}

#[test]
fn a_fake_slot_hashes_account_is_refused() {
    let mut w = world();
    let round = opened_round(&mut w, 9, 3);
    at_slot(&mut w.svm, COMMIT + 100, &recent(COMMIT + 100, &[]));
    let (anyone, _) = newcomer(&mut w.svm, 0);
    let mut ix = reveal_ix(round, w.depositor.pubkey(), hex32(VECTOR_SEED));
    ix.accounts[2].pubkey = Address::new_unique();
    let err = try_ix(&mut w.svm, ix, &anyone).unwrap_err();
    assert!(err.contains("ConstraintAddress"), "{err}");
}

#[test]
fn a_round_must_be_drawable() {
    let mut w = world();
    let d = w.depositor.insecure_clone();
    for (n, size, count) in [(10u8, 0u32, 0u16), (11, 5, 0), (12, 5, 6), (13, MAX_ROUND_CANDIDATES + 1, 1)] {
        let err = try_ix(&mut w.svm, open_round_ix(d.pubkey(), list(n), size, count, [0u8; 32]), &d).unwrap_err();
        assert!(err.contains("BadRound"), "size {size} count {count}: {err}");
    }
}

#[test]
fn reading_slot_hashes_finds_the_right_entry() {
    use sasona::{entropy_for, Entropy};
    let bytes = |entries: &[(u64, [u8; 32])]| {
        let mut d = (entries.len() as u64).to_le_bytes().to_vec();
        for (s, h) in entries {
            d.extend_from_slice(&s.to_le_bytes());
            d.extend_from_slice(h);
        }
        d
    };
    let data = bytes(&[(110, slot_hash(110)), (105, slot_hash(105)), (101, slot_hash(101)), (99, slot_hash(99))]);
    assert!(matches!(entropy_for(&data, 100).unwrap(), Entropy::Found(101, h) if h == slot_hash(101)));
    assert!(matches!(entropy_for(&data, 105).unwrap(), Entropy::Found(105, _)));
    assert!(matches!(entropy_for(&data, 111).unwrap(), Entropy::TooEarly));
    assert!(matches!(entropy_for(&data, 98).unwrap(), Entropy::Gone));
    assert!(entropy_for(&data[..20], 100).is_err(), "truncated data is refused");
}

#[test]
fn a_list_can_be_drawn_once() {
    // Otherwise an opener could open several rounds for one list, reveal them
    // all, and keep the result they liked.
    let mut w = world();
    opened_round(&mut w, 20, 3);
    let (other, _) = newcomer(&mut w.svm, 0);
    for (who, count) in [(w.depositor.insecure_clone(), 3u16), (w.depositor.insecure_clone(), 2), (other, 3)] {
        let err = try_ix(&mut w.svm, open_round_ix(who.pubkey(), list(20), 40, count, [5u8; 32]), &who).unwrap_err();
        assert!(err.contains("already in use"), "{err}");
    }
}

#[test]
fn a_withheld_round_cannot_be_revealed_and_a_drawn_one_cannot_be_withheld() {
    let mut w = world();
    let (anyone, _) = newcomer(&mut w.svm, 0);
    let opener = w.depositor.pubkey();
    let target = COMMIT + ENTROPY_DELAY_SLOTS;

    let drawn = opened_round(&mut w, 21, 3);
    at_slot(&mut w.svm, COMMIT + 100, &recent(COMMIT + 100, &[]));
    try_ix(&mut w.svm, reveal_ix(drawn, opener, hex32(VECTOR_SEED)), &anyone).unwrap();
    at_slot(&mut w.svm, target + 600, &recent(target + 600, &[]));
    let err = try_ix(&mut w.svm, withheld_ix(drawn), &anyone).unwrap_err();
    assert!(err.contains("RoundNotOpen"), "{err}");

    let kept = opened_round(&mut w, 22, 3);
    at_slot(&mut w.svm, target + 600, &recent(target + 600, &[]));
    try_ix(&mut w.svm, withheld_ix(kept), &anyone).unwrap();
    let err = try_ix(&mut w.svm, reveal_ix(kept, opener, hex32(VECTOR_SEED)), &anyone).unwrap_err();
    assert!(err.contains("RoundNotOpen"), "{err}");
}

#[test]
fn the_real_slot_hashes_encoding_is_read_correctly() {
    // Built by the Solana crate that defines the sysvar, not by our own
    // assumption of its layout.
    use sasona::{entropy_for, Entropy};
    let entries: Vec<(u64, solana_hash::Hash)> =
        (90..120u64).rev().filter(|s| *s != 100).map(|s| (s, solana_hash::Hash::new_from_array(slot_hash(s)))).collect();
    let real = solana_slot_hashes::SlotHashes::new(&entries);
    let data = bincode::serialize(&real).unwrap();
    assert!(matches!(entropy_for(&data, 100).unwrap(), Entropy::Found(101, h) if h == slot_hash(101)));
    assert!(matches!(entropy_for(&data, 90).unwrap(), Entropy::Found(90, _)), "the oldest entry is still usable");
    assert!(matches!(entropy_for(&data, 89).unwrap(), Entropy::Gone));
    assert!(matches!(entropy_for(&data, 120).unwrap(), Entropy::TooEarly));
    let empty = bincode::serialize(&solana_slot_hashes::SlotHashes::new(&[])).unwrap();
    assert!(entropy_for(&empty, 100).is_err(), "no entries at all is refused");
}

#[test]
fn revealing_at_the_far_end_of_the_window_fits_easily_in_a_transaction() {
    let mut w = world();
    let round = opened_round(&mut w, 23, 3);
    let target = COMMIT + ENTROPY_DELAY_SLOTS;
    // The target is the oldest entry left, so the scan walks all 512.
    at_slot(&mut w.svm, target + 512, &recent(target + 512, &[]));
    let (anyone, _) = newcomer(&mut w.svm, 0);
    w.svm.expire_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[reveal_ix(round, w.depositor.pubkey(), hex32(VECTOR_SEED))],
        Some(&anyone.pubkey()),
        &[&anyone],
        w.svm.latest_blockhash(),
    );
    let used = w.svm.send_transaction(tx).unwrap().compute_units_consumed;
    // A transaction gets 200,000 by default. Half of that leaves room.
    assert!(used < 100_000, "used {used} compute units");
    let r: Round = read(&w.svm, round);
    assert_eq!(r.entropy_slot, target);
}
