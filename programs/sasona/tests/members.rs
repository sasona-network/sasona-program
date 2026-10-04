//! Members: a stake locked per membership, the reader of each service drawn
//! from the round's roster, and leaving after notice. Run against the
//! compiled program.

mod common;
use common::*;
use sasona::{canonical_question, reader_number, Round, MEMBER_ACTIVE, MEMBER_LEAVING, MEMBER_LEFT, MEMBER_STAKE};

const SERVICE: &str = "https://sandbox.example.net/run/python";

fn hex32(s: &str) -> [u8; 32] {
    let v: Vec<u8> = (0..32).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect();
    v.try_into().unwrap()
}

fn coins(svm: &LiteSVM, who: Address) -> u64 {
    token_balance(svm, ata(who, pda(&[COIN_SEED])))
}

/// A round over a list with fingerprint `[fp; 32]`, opened by `who` at
/// `slot` and drawn 40 slots later.
fn drawn_round(w: &mut World, who: &Keypair, fp: u8, slot: u64) -> Address {
    let seed = [fp; 32];
    w.svm.warp_to_slot(slot);
    try_ix(&mut w.svm, open_round_ix(who.pubkey(), [fp; 32], 10, 2, sha256(&seed)), who).unwrap();
    at_slot(&mut w.svm, slot + 40, &recent(slot + 40, &[]));
    try_ix(&mut w.svm, reveal_ix(round_address([fp; 32]), who.pubkey(), seed), who).unwrap();
    round_address([fp; 32])
}

fn commit(w: &mut World, who: &Keypair, round: Address, service: &str) -> Result<(), String> {
    let q = sha256(&canonical_question(&[7u8; 16]));
    try_with(&mut w.svm, |s| commit_reading_ix(s, who.pubkey(), round, service, q), who)
}

#[test]
fn the_program_draws_readers_as_the_protocol_does() {
    // Every draw in sasona-protocol's vectors/reader.json, 0.5.0.
    const PRICES: &str = "https://data.example.com/prices";
    for (seed, service, members, want) in [
        ("7692c3ad3540bb803c020b3aee66cd8887123234ea0c6e7143c0add73ff431ed", SERVICE, 1, [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]),
        ("222b0bd51fcef7e65c2e62db2ed65457013bab56be6fafeb19ee11d453153c80", SERVICE, 5, [5, 3, 4, 4, 4, 3, 3, 3, 4, 4, 3, 4, 2, 4, 4, 4]),
        ("222b0bd51fcef7e65c2e62db2ed65457013bab56be6fafeb19ee11d453153c80", PRICES, 5, [2, 2, 3, 1, 5, 3, 1, 2, 4, 1, 1, 5, 1, 3, 3, 3]),
        ("8b5b9db0c13db24256c829aa364aa90c6d2eba318b9232a4ab9313b954d3555f", SERVICE, 5, [4, 5, 1, 1, 5, 5, 1, 1, 4, 5, 5, 5, 1, 5, 3, 3]),
        ("4fb62348858c2f6fbd6db27fa4c11edd8559119869d1034e2bd3390fd92b1a04", SERVICE, 5, [3, 1, 2, 3, 4, 1, 1, 4, 4, 1, 5, 3, 1, 5, 4, 5]),
        ("f905b19542ed08c9a9c26543cca32e5711d207dcffb81b4cdb44ce0b989431c9", SERVICE, 2, [1, 2, 1, 1, 2, 2, 1, 1, 1, 1, 2, 2, 2, 1, 2, 1]),
        ("9e153632d2f4c4e1611a227269d03fe28eed0b7e527a68dc3d0afba3bcac09a8", SERVICE, 5, [3, 4, 5, 4, 5, 2, 4, 4, 2, 4, 1, 2, 1, 5, 1, 1]),
        ("f3ff8de6341ec9d297748044ba572a388e75e7b97aef535560f969500d44b16a", SERVICE, 5, [4, 5, 1, 4, 1, 2, 2, 3, 2, 1, 2, 3, 4, 4, 4, 1]),
    ] {
        let got: Vec<u32> = (0..16).map(|a| reader_number(&hex32(seed), &sha256(service.as_bytes()), a, members)).collect();
        assert_eq!(got, want, "{seed} {service}");
    }
}

#[test]
fn a_membership_locks_one_stake() {
    let mut w = world();
    open(&mut w, 1_000 * DOLLAR);
    let d = w.depositor.insecure_clone();
    let before = coins(&w.svm, d.pubkey());
    assert_eq!(join(&mut w.svm, &d), 1);
    assert_eq!(coins(&w.svm, d.pubkey()), before - MEMBER_STAKE);
    assert_eq!(token_balance(&w.svm, pda(&[sasona::STAKES_SEED])), MEMBER_STAKE);
    let m = member(&w.svm, 1);
    assert_eq!((addr(m.owner), m.number, m.state, m.stake, m.open_challenges), (d.pubkey(), 1, MEMBER_ACTIVE, MEMBER_STAKE, 0));

    // One key may hold several. Each is one more stake and one more chance.
    assert_eq!(join(&mut w.svm, &d), 2);
    assert_eq!(token_balance(&w.svm, pda(&[sasona::STAKES_SEED])), 2 * MEMBER_STAKE);
    assert_eq!(member_count(&w.svm), 2);
}

#[test]
fn a_membership_needs_the_whole_stake() {
    let mut w = world();
    open(&mut w, 1_000 * DOLLAR);
    let (k, usd) = newcomer(&mut w.svm, 1);
    try_deposit(&mut w.svm, &k, usd, DOLLAR).unwrap();
    assert!(coins(&w.svm, k.pubkey()) < MEMBER_STAKE);
    let err = try_ix(&mut w.svm, join_members_ix(k.pubkey(), 1), &k).unwrap_err();
    assert!(err.contains("insufficient funds"), "{err}");
    assert_eq!(member_count(&w.svm), 0);
}

#[test]
fn a_membership_cannot_skip_a_number() {
    let mut w = world_with_member();
    let (k, usd) = newcomer(&mut w.svm, 100);
    try_deposit(&mut w.svm, &k, usd, 100 * DOLLAR).unwrap();
    let err = try_ix(&mut w.svm, join_members_ix(k.pubkey(), 3), &k).unwrap_err();
    assert!(err.contains("ConstraintSeeds"), "{err}");
}

#[test]
fn leaving_takes_notice() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    // A stranger with coins of their own, so only ownership is in question.
    let (stranger, usd) = newcomer(&mut w.svm, 10);
    try_deposit(&mut w.svm, &stranger, usd, 10 * DOLLAR).unwrap();
    let err = try_ix(&mut w.svm, ask_to_leave_ix(stranger.pubkey(), 1), &stranger).unwrap_err();
    assert!(err.contains("NotTheOwner"), "{err}");
    let err = try_ix(&mut w.svm, leave_ix(d.pubkey(), 1), &d).unwrap_err();
    assert!(err.contains("NotLeaving"), "{err}");

    let before = coins(&w.svm, d.pubkey());
    try_ix(&mut w.svm, ask_to_leave_ix(d.pubkey(), 1), &d).unwrap();
    let now = w.svm.get_sysvar::<solana_clock::Clock>().unix_timestamp;
    let m = member(&w.svm, 1);
    assert_eq!((m.state, m.leave_at), (MEMBER_LEAVING, now + NOTICE_SECONDS));
    let err = try_ix(&mut w.svm, ask_to_leave_ix(d.pubkey(), 1), &d).unwrap_err();
    assert!(err.contains("NotActive"), "{err}");

    days_pass(&mut w.svm, 44);
    let err = try_ix(&mut w.svm, leave_ix(d.pubkey(), 1), &d).unwrap_err();
    assert!(err.contains("NoticeNotOver"), "{err}");
    days_pass(&mut w.svm, 1);
    let err = try_ix(&mut w.svm, leave_ix(stranger.pubkey(), 1), &stranger).unwrap_err();
    assert!(err.contains("NotTheOwner"), "{err}");
    try_ix(&mut w.svm, leave_ix(d.pubkey(), 1), &d).unwrap();
    assert_eq!(coins(&w.svm, d.pubkey()), before + MEMBER_STAKE);
    let m = member(&w.svm, 1);
    assert_eq!((m.state, m.stake), (MEMBER_LEFT, 0));
    let err = try_ix(&mut w.svm, leave_ix(d.pubkey(), 1), &d).unwrap_err();
    assert!(err.contains("NotLeaving"), "{err}");
}

#[test]
fn only_the_drawn_member_reads() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let (b, _) = new_member(&mut w.svm);
    let round = drawn_round(&mut w, &d, 1, 1_000);
    let r: Round = read(&w.svm, round);
    assert_eq!(r.members, 2);
    let (drawn, _) = drawn_for(&w.svm, round, SERVICE, None);
    let (yes, no) = if drawn == Some(1) { (&d, &b) } else { (&b, &d) };
    let err = commit(&mut w, no, round, SERVICE).unwrap_err();
    assert!(err.contains("NotDrawn"), "{err}");
    commit(&mut w, yes, round, SERVICE).unwrap();
    let reading: sasona::Reading = read(&w.svm, reading_address(round, SERVICE));
    assert_eq!(reading.member, drawn.unwrap());
}

/// A round where membership `first` is drawn first for SERVICE, among
/// `members`, found by trying fingerprints.
fn round_drawing_first(w: &mut World, opener: &Keypair, members: u32, first: u32) -> Address {
    for fp in 100..=255u8 {
        let slot = 1_000 + fp as u64 * 100;
        let round = drawn_round(w, opener, fp, slot);
        let r: Round = read(&w.svm, round);
        assert_eq!(r.members, members);
        if reader_number(&r.final_seed, &sha256(SERVICE.as_bytes()), 0, members) == first {
            return round;
        }
    }
    panic!("no fingerprint drew membership {first} first");
}

#[test]
fn an_active_member_cannot_be_passed_over() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let (b, _) = new_member(&mut w.svm);
    let round = round_drawing_first(&mut w, &d, 2, 1);
    // b claims to read after passing over membership 1, which is active.
    let q = sha256(&canonical_question(&[7u8; 16]));
    let mut ix = commit_reading_ix(&w.svm, b.pubkey(), round, SERVICE, q);
    ix.accounts[2].pubkey = member_address(2);
    ix.accounts.push(AccountMeta::new_readonly(member_address(1), false));
    let err = try_ix(&mut w.svm, ix, &b).unwrap_err();
    assert!(err.contains("NotSkippable"), "{err}");
}

#[test]
fn the_memberships_passed_over_must_be_the_ones_drawn() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let (b, _) = new_member(&mut w.svm);
    let (c, _) = new_member(&mut w.svm);
    let round = round_drawing_first(&mut w, &d, 3, 1);
    // Membership 3 is inactive, but it is not the one drawn first.
    try_ix(&mut w.svm, ask_to_leave_ix(c.pubkey(), 3), &c).unwrap();
    let q = sha256(&canonical_question(&[7u8; 16]));
    let mut ix = commit_reading_ix(&w.svm, b.pubkey(), round, SERVICE, q);
    ix.accounts[2].pubkey = member_address(2);
    ix.accounts.push(AccountMeta::new_readonly(member_address(3), false));
    let err = try_ix(&mut w.svm, ix, &b).unwrap_err();
    assert!(err.contains("NotDrawn"), "{err}");

    // Nor can something that is not a membership stand in for one.
    let mut ix = commit_reading_ix(&w.svm, b.pubkey(), round, SERVICE, q);
    ix.accounts[2].pubkey = member_address(2);
    ix.accounts.push(AccountMeta::new_readonly(round, false));
    let err = try_ix(&mut w.svm, ix, &b).unwrap_err();
    assert!(err.contains("AccountDiscriminatorMismatch"), "{err}");
}

#[test]
fn a_member_who_asked_to_leave_is_passed_over() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let (b, _) = new_member(&mut w.svm);
    let round = round_drawing_first(&mut w, &d, 2, 1);
    try_ix(&mut w.svm, ask_to_leave_ix(d.pubkey(), 1), &d).unwrap();
    let (drawn, skipped) = drawn_for(&w.svm, round, SERVICE, None);
    assert_eq!((drawn, skipped.first().copied()), (Some(2), Some(1)));
    let err = commit(&mut w, &d, round, SERVICE).unwrap_err();
    assert!(err.contains("NotDrawn"), "{err}");
    commit(&mut w, &b, round, SERVICE).unwrap();
}

#[test]
fn a_membership_taken_after_the_round_opened_is_not_on_its_roster() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let round = drawn_round(&mut w, &d, 1, 1_000);
    let (b, _) = new_member(&mut w.svm);
    let r: Round = read(&w.svm, round);
    assert_eq!(r.members, 1);
    let err = commit(&mut w, &b, round, SERVICE).unwrap_err();
    assert!(err.contains("NotDrawn"), "{err}");
    commit(&mut w, &d, round, SERVICE).unwrap();
}

#[test]
fn a_round_opened_before_anyone_joined_has_no_readers() {
    let mut w = world();
    open(&mut w, 1_000 * DOLLAR);
    let d = w.depositor.insecure_clone();
    let round = drawn_round(&mut w, &d, 1, 1_000);
    join(&mut w.svm, &d);
    let err = commit(&mut w, &d, round, SERVICE).unwrap_err();
    assert!(err.contains("NotDrawn"), "{err}");
}

#[test]
fn a_second_reading_passes_over_every_membership_of_the_first_reader() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let first_round = drawn_round(&mut w, &d, 1, 1_000);
    commit(&mut w, &d, first_round, SERVICE).unwrap();
    let first = reading_address(first_round, SERVICE);
    w.svm.warp_to_slot(1_050);
    try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), first, [7u8; 16], [1u8; 32], 1), &d).unwrap();

    // The first reader now holds memberships 1 and 2, and b holds 3.
    join(&mut w.svm, &d);
    let (b, _) = new_member(&mut w.svm);
    let reread = drawn_round(&mut w, &b, 2, 1_100);
    let (drawn, skipped) = drawn_for(&w.svm, reread, SERVICE, Some(d.pubkey()));
    assert_eq!(drawn, Some(3));
    assert!(skipped.iter().all(|&k| k == 1 || k == 2), "{skipped:?}");
    let q = sha256(&canonical_question(&[8u8; 16]));
    try_with(&mut w.svm, |s| commit_second_ix(s, b.pubkey(), reread, SERVICE, q, first), &b).unwrap();
}
