//! Members: a stake locked per membership, a roster of seats with no gaps,
//! the reader of each service drawn from it, and leaving after notice. Run
//! against the compiled program.

mod common;
use common::*;
use sasona::{canonical_question, reader_number, Reading, Round, MEMBER_ACTIVE, MEMBER_LEAVING, MEMBER_LEFT, MEMBER_STAKE, READ_WINDOW_SLOTS};

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

/// Seat k holds membership `number`, and the membership knows it.
fn assert_seat(svm: &LiteSVM, k: u32, number: u32) {
    let s = seat(svm, k);
    assert_eq!(s.member, number, "seat {k}");
    if number > 0 {
        let m = member(svm, number);
        assert_eq!((m.seat, s.owner), (k, m.owner), "membership {number}");
    }
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
        ("283bb9deef02e6843abfb538efa1eca70801bd8a701c3f98191e123496339247", SERVICE, 5, [3, 1, 5, 1, 3, 4, 3, 1, 1, 3, 5, 5, 1, 2, 2, 5]),
        ("f905b19542ed08c9a9c26543cca32e5711d207dcffb81b4cdb44ce0b989431c9", SERVICE, 2, [1, 2, 1, 1, 2, 2, 1, 1, 1, 1, 2, 2, 2, 1, 2, 1]),
        ("bc00525689a7313aaf2528060ddf7bea2701f5e40338d52a5ca27f55ffdb155e", SERVICE, 5, [5, 4, 3, 1, 4, 4, 1, 5, 3, 3, 2, 2, 2, 1, 4, 3]),
        ("36211a7a8b0e332b15fc6b08f479dfb0e283ea11b0bafbbffd7ed07358d6c569", SERVICE, 5, [3, 5, 1, 2, 3, 4, 2, 3, 2, 3, 5, 3, 5, 4, 1, 2]),
        ("08e7e6e4c507a9b273cfabfe2be49eedda26c7134e99bf4534c19df1786f6617", SERVICE, 3, [1, 2, 1, 2, 2, 3, 2, 1, 3, 2, 3, 1, 2, 1, 2, 3]),
    ] {
        let got: Vec<u32> = (0..16).map(|a| reader_number(&hex32(seed), &sha256(service.as_bytes()), a, members)).collect();
        assert_eq!(got, want, "{seed} {service}");
    }
}

#[test]
fn a_membership_locks_one_stake_and_takes_the_next_seat() {
    let mut w = world();
    open(&mut w, 1_000 * DOLLAR);
    let d = w.depositor.insecure_clone();
    let before = coins(&w.svm, d.pubkey());
    assert_eq!(join(&mut w.svm, &d), 1);
    assert_eq!(coins(&w.svm, d.pubkey()), before - MEMBER_STAKE);
    assert_eq!(token_balance(&w.svm, pda(&[sasona::STAKES_SEED])), MEMBER_STAKE);
    let m = member(&w.svm, 1);
    assert_eq!((addr(m.owner), m.number, m.state, m.stake, m.open_challenges), (d.pubkey(), 1, MEMBER_ACTIVE, MEMBER_STAKE, 0));
    assert_seat(&w.svm, 1, 1);

    // One key may hold several. Each is one more stake and one more seat.
    assert_eq!(join(&mut w.svm, &d), 2);
    assert_seat(&w.svm, 2, 2);
    assert_eq!(token_balance(&w.svm, pda(&[sasona::STAKES_SEED])), 2 * MEMBER_STAKE);
    assert_eq!((member_count(&w.svm), seated(&w.svm)), (2, 2));
}

#[test]
fn a_membership_needs_the_whole_stake() {
    let mut w = world();
    open(&mut w, 1_000 * DOLLAR);
    let (k, usd) = newcomer(&mut w.svm, 1);
    try_deposit(&mut w.svm, &k, usd, DOLLAR).unwrap();
    assert!(coins(&w.svm, k.pubkey()) < MEMBER_STAKE);
    let err = try_ix(&mut w.svm, join_members_ix(k.pubkey(), 1, 1), &k).unwrap_err();
    assert!(err.contains("insufficient funds"), "{err}");
    assert_eq!(member_count(&w.svm), 0);
}

#[test]
fn a_membership_cannot_pick_its_number_or_its_seat() {
    let mut w = world_with_member();
    let (k, usd) = newcomer(&mut w.svm, 100);
    try_deposit(&mut w.svm, &k, usd, 100 * DOLLAR).unwrap();
    let err = try_ix(&mut w.svm, join_members_ix(k.pubkey(), 3, 2), &k).unwrap_err();
    assert!(err.contains("ConstraintSeeds"), "{err}");
    let err = try_ix(&mut w.svm, join_members_ix(k.pubkey(), 2, 3), &k).unwrap_err();
    assert!(err.contains("ConstraintSeeds"), "{err}");
}

#[test]
fn leaving_takes_notice() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    // A stranger with coins of their own, so only ownership is in question.
    let (stranger, usd) = newcomer(&mut w.svm, 10);
    try_deposit(&mut w.svm, &stranger, usd, 10 * DOLLAR).unwrap();
    let err = try_with(&mut w.svm, |s| ask_to_leave_ix(s, stranger.pubkey(), 1), &stranger).unwrap_err();
    assert!(err.contains("NotTheOwner"), "{err}");
    let err = try_ix(&mut w.svm, leave_ix(d.pubkey(), 1), &d).unwrap_err();
    assert!(err.contains("NotLeaving"), "{err}");

    let before = coins(&w.svm, d.pubkey());
    try_with(&mut w.svm, |s| ask_to_leave_ix(s, d.pubkey(), 1), &d).unwrap();
    let now = w.svm.get_sysvar::<solana_clock::Clock>().unix_timestamp;
    let m = member(&w.svm, 1);
    assert_eq!((m.state, m.leave_at, m.seat), (MEMBER_LEAVING, now + NOTICE_SECONDS, 0));
    assert_eq!(seated(&w.svm), 0);
    assert_seat(&w.svm, 1, 0);
    let err = try_with(&mut w.svm, |s| ask_to_leave_ix(s, d.pubkey(), 1), &d).unwrap_err();
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
fn the_roster_has_no_gaps() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let (b, _) = new_member(&mut w.svm);
    let (_c, _) = new_member(&mut w.svm);
    // b leaves the middle seat: the last membership moves into it.
    try_with(&mut w.svm, |s| ask_to_leave_ix(s, b.pubkey(), 2), &b).unwrap();
    assert_eq!(seated(&w.svm), 2);
    assert_seat(&w.svm, 1, 1);
    assert_seat(&w.svm, 2, 3);
    assert_seat(&w.svm, 3, 0);
    assert_eq!(member(&w.svm, 2).seat, 0);

    // The next to join takes seat 3 again, with a new number.
    let (e, n) = new_member(&mut w.svm);
    assert_eq!(n, 4);
    assert_seat(&w.svm, 3, 4);
    // The first seat empties: membership 4, sitting last, moves into it.
    try_with(&mut w.svm, |s| ask_to_leave_ix(s, d.pubkey(), 1), &d).unwrap();
    assert_seat(&w.svm, 1, 4);
    assert_seat(&w.svm, 2, 3);
    assert_seat(&w.svm, 3, 0);
    assert_eq!(addr(seat(&w.svm, 1).owner), e.pubkey());
}

#[test]
fn the_last_seat_leaving_only_empties_it() {
    let mut w = world_with_member();
    let (b, _) = new_member(&mut w.svm);
    try_with(&mut w.svm, |s| ask_to_leave_ix(s, b.pubkey(), 2), &b).unwrap();
    assert_eq!(seated(&w.svm), 1);
    assert_seat(&w.svm, 1, 1);
    assert_seat(&w.svm, 2, 0);
    // A round opened now is drawn over the one seat left, not the two ever taken.
    let d = w.depositor.insecure_clone();
    let round = drawn_round(&mut w, &d, 1, 1_000);
    let r: Round = read(&w.svm, round);
    assert_eq!(r.members, 1);
}

#[test]
fn leaving_needs_the_seats_that_actually_move() {
    let mut w = world_with_member();
    let (b, _) = new_member(&mut w.svm);
    let (_c, _) = new_member(&mut w.svm);
    // Name membership 1 as the one sitting last, which it is not.
    let mut ix = ask_to_leave_ix(&w.svm, b.pubkey(), 2);
    let last = ix.accounts.len() - 1;
    ix.accounts[last].pubkey = member_address(1);
    let err = try_ix(&mut w.svm, ix, &b).unwrap_err();
    assert!(err.contains("NotTheSeat"), "{err}");
    // Or leave out the last seat altogether.
    let mut ix = ask_to_leave_ix(&w.svm, b.pubkey(), 2);
    ix.accounts.truncate(ix.accounts.len() - 2);
    let err = try_ix(&mut w.svm, ix, &b).unwrap_err();
    assert!(err.contains("NotTheSeat"), "{err}");
    assert_eq!(seated(&w.svm), 3);
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
    assert!(err.contains("NotSkippable") || err.contains("NotDrawn"), "{err}");
    commit(&mut w, yes, round, SERVICE).unwrap();
    let reading: Reading = read(&w.svm, reading_address(round, SERVICE));
    assert_eq!(reading.member, seat(&w.svm, drawn.unwrap()).member);
}

/// A round where seat `first` is drawn first for SERVICE, out of `members`,
/// found by trying fingerprints.
fn round_drawing_first(w: &mut World, opener: &Keypair, members: u32, first: u32) -> Address {
    for fp in 100..=255u8 {
        let slot = 1_000 + fp as u64 * 100;
        let round = drawn_round(w, opener, fp, slot);
        let r: Round = read(&w.svm, round);
        assert_eq!(r.members, members);
        let h = sha256(SERVICE.as_bytes());
        if reader_number(&r.final_seed, &h, 0, members) == first && reader_number(&r.final_seed, &h, 1, members) != first {
            return round;
        }
    }
    panic!("no fingerprint drew seat {first} first");
}

#[test]
fn a_seat_drawn_first_cannot_be_passed_over() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let (b, _) = new_member(&mut w.svm);
    let round = round_drawing_first(&mut w, &d, 2, 1);
    // b claims the reading, with or without showing seat 1, which d holds.
    let err = commit(&mut w, &b, round, SERVICE).unwrap_err();
    assert!(err.contains("NotSkippable"), "{err}");
    let q = sha256(&canonical_question(&[7u8; 16]));
    let mut ix = commit_reading_ix(&w.svm, b.pubkey(), round, SERVICE, q);
    ix.accounts.push(AccountMeta::new_readonly(seat_address(1), false));
    let err = try_ix(&mut w.svm, ix, &b).unwrap_err();
    assert!(err.contains("NotSkippable"), "{err}");
}

#[test]
fn the_seats_shown_must_be_the_ones_drawn() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let (b, _) = new_member(&mut w.svm);
    let (_c, _) = new_member(&mut w.svm);
    let round = round_drawing_first(&mut w, &d, 3, 1);
    let q = sha256(&canonical_question(&[7u8; 16]));
    let mut ix = commit_reading_ix(&w.svm, b.pubkey(), round, SERVICE, q);
    ix.accounts.push(AccountMeta::new_readonly(seat_address(3), false));
    let err = try_ix(&mut w.svm, ix, &b).unwrap_err();
    assert!(err.contains("NotTheSeat"), "{err}");

    // Nor can something that is not a seat stand in for one.
    let mut ix = commit_reading_ix(&w.svm, b.pubkey(), round, SERVICE, q);
    ix.accounts.push(AccountMeta::new_readonly(round, false));
    let err = try_ix(&mut w.svm, ix, &b).unwrap_err();
    assert!(err.contains("AccountDiscriminatorMismatch"), "{err}");

    // And the member drawn cannot bring along seats nobody passed over.
    let mut ix = commit_reading_ix(&w.svm, d.pubkey(), round, SERVICE, q);
    ix.accounts.push(AccountMeta::new_readonly(seat_address(2), false));
    let err = try_ix(&mut w.svm, ix, &d).unwrap_err();
    assert!(err.contains("NotDrawn"), "{err}");
    commit(&mut w, &d, round, SERVICE).unwrap();
}

#[test]
fn a_member_who_moves_up_after_the_draw_does_not_read_for_that_round() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let (b, _) = new_member(&mut w.svm);
    let round = round_drawing_first(&mut w, &d, 2, 1);
    try_with(&mut w.svm, |s| ask_to_leave_ix(s, d.pubkey(), 1), &d).unwrap();
    assert_seat(&w.svm, 1, 2);
    // d has left; b sits in seat 1 now, but only since the round.
    let err = commit(&mut w, &d, round, SERVICE).unwrap_err();
    assert!(err.contains("ConstraintSeeds") || err.contains("NotActive"), "{err}");
    let err = commit(&mut w, &b, round, SERVICE).unwrap_err();
    assert!(err.contains("SatDownSince"), "{err}");
    assert_eq!(drawn_for(&w.svm, round, SERVICE, None).0, None);
    // For a round committed after the move, b reads.
    let later = drawn_round(&mut w, &b, 50, 30_000);
    commit(&mut w, &b, later, SERVICE).unwrap();
}

#[test]
fn a_seat_no_longer_on_the_roster_is_passed_over() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let (b, _) = new_member(&mut w.svm);
    let (c, _) = new_member(&mut w.svm);
    let round = round_drawing_first(&mut w, &d, 3, 3);
    try_with(&mut w.svm, |s| ask_to_leave_ix(s, c.pubkey(), 3), &c).unwrap();
    let (drawn, skipped) = drawn_for(&w.svm, round, SERVICE, None);
    assert!(drawn.unwrap() <= 2 && skipped.is_empty());
    let who = if seat(&w.svm, drawn.unwrap()).owner == key(d.pubkey()) { &d } else { &b };
    commit(&mut w, who, round, SERVICE).unwrap();
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
    assert!(err.contains("SatDownSince"), "{err}");
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
fn a_reading_is_committed_within_an_hour_of_the_draw() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let round = drawn_round(&mut w, &d, 1, 1_000);
    let r: Round = read(&w.svm, round);
    w.svm.warp_to_slot(r.entropy_slot + READ_WINDOW_SLOTS + 1);
    let err = commit(&mut w, &d, round, SERVICE).unwrap_err();
    assert!(err.contains("WindowClosed"), "{err}");
    w.svm.warp_to_slot(r.entropy_slot + READ_WINDOW_SLOTS);
    commit(&mut w, &d, round, "https://data.example.com/prices").unwrap();
}

#[test]
fn a_second_reading_passes_over_every_seat_of_the_first_reader() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let first_round = drawn_round(&mut w, &d, 1, 1_000);
    commit(&mut w, &d, first_round, SERVICE).unwrap();
    let first = reading_address(first_round, SERVICE);
    w.svm.warp_to_slot(1_050);
    try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), first, [7u8; 16], [1u8; 32], 1), &d).unwrap();

    // The first reader now holds seats 1 and 2, and b holds 3.
    join(&mut w.svm, &d);
    let (b, _) = new_member(&mut w.svm);
    let reread = drawn_round(&mut w, &b, 2, 1_100);
    let (drawn, skipped) = drawn_for(&w.svm, reread, SERVICE, Some(d.pubkey()));
    assert_eq!(drawn, Some(3));
    assert!(skipped.iter().all(|&k| k == 1 || k == 2), "{skipped:?}");
    let q = sha256(&canonical_question(&[8u8; 16]));
    try_with(&mut w.svm, |s| commit_second_ix(s, b.pubkey(), reread, SERVICE, q, first), &b).unwrap();
}

#[test]
fn a_reading_from_before_members_can_still_be_re_read() {
    // Readings already on devnet were made when the account had no
    // membership or reveal time. Their padding reads as zero.
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let first_round = drawn_round(&mut w, &d, 1, 1_000);
    commit(&mut w, &d, first_round, SERVICE).unwrap();
    let first = reading_address(first_round, SERVICE);
    w.svm.warp_to_slot(1_050);
    try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), first, [7u8; 16], [1u8; 32], 1), &d).unwrap();
    let mut acc = w.svm.get_account(&first).unwrap();
    let used = 8 + 32 + 4 + SERVICE.len() + 32 + 32 + 8 + 1 + 32 + 1 + 8 + 1;
    acc.data.truncate(415);
    for b in acc.data[used..].iter_mut() {
        *b = 0;
    }
    w.svm.set_account(first, acc).unwrap();
    let old: Reading = read(&w.svm, first);
    assert_eq!((old.member, old.reveal_time, old.verdict), (0, 0, 1));

    let (b, _) = new_member(&mut w.svm);
    let reread = drawn_round(&mut w, &b, 2, 1_100);
    let q = sha256(&canonical_question(&[8u8; 16]));
    try_with(&mut w.svm, |s| commit_second_ix(s, b.pubkey(), reread, SERVICE, q, first), &b).unwrap();
    // It has no stake behind it, so there is nothing to challenge.
    let (c, _) = newcomer(&mut w.svm, 0);
    let err = try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), first), &c).unwrap_err();
    assert!(err.contains("NoMember") || err.contains("AccountNotInitialized"), "{err}");
}

#[test]
fn a_membership_that_sat_down_after_the_round_does_not_read_for_it() {
    // The attack the second review found: leave, rejoin into the last seat,
    // and read whatever was drawn to it.
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let (b, _) = new_member(&mut w.svm);
    let (c, _) = new_member(&mut w.svm);
    let round = round_drawing_first(&mut w, &d, 3, 3);
    // b leaves seat 2, so c moves up into it; then b joins again into seat 3.
    try_with(&mut w.svm, |s| ask_to_leave_ix(s, b.pubkey(), 2), &b).unwrap();
    assert_seat(&w.svm, 2, 3);
    join(&mut w.svm, &b);
    assert_seat(&w.svm, 3, 4);
    let err = commit(&mut w, &b, round, SERVICE).unwrap_err();
    assert!(err.contains("SatDownSince"), "{err}");

    // The draw passes over both seats sat in since; the reader is whoever is
    // left of those who were seated when the round was committed.
    let (drawn, skipped) = drawn_for(&w.svm, round, SERVICE, None);
    if let Some(k) = drawn {
        assert_eq!(k, 1);
        assert!(skipped.iter().all(|&s| s == 2 || s == 3), "{skipped:?}");
        commit(&mut w, &d, round, SERVICE).unwrap();
    }
    let _ = c;
}

#[test]
fn a_seat_sat_in_since_the_round_must_be_shown_to_be_passed_over() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let (b, _) = new_member(&mut w.svm);
    let round = round_drawing_first(&mut w, &d, 2, 2);
    // b leaves the last seat and rejoins: seat 2 is sat in since the round.
    try_with(&mut w.svm, |s| ask_to_leave_ix(s, b.pubkey(), 2), &b).unwrap();
    join(&mut w.svm, &b);
    let q = sha256(&canonical_question(&[7u8; 16]));
    let mut ix = commit_reading_ix(&w.svm, d.pubkey(), round, SERVICE, q);
    let shown = ix.accounts.len() - 1;
    assert_eq!(ix.accounts[shown].pubkey, seat_address(2));
    ix.accounts.truncate(shown);
    let err = try_ix(&mut w.svm, ix, &d).unwrap_err();
    assert!(err.contains("NotSkippable"), "{err}");
    commit(&mut w, &d, round, SERVICE).unwrap();
}

#[test]
fn a_member_who_joined_in_the_rounds_own_slot_does_not_read_for_it() {
    let mut w = world();
    open(&mut w, 1_000 * DOLLAR);
    let d = w.depositor.insecure_clone();
    w.svm.warp_to_slot(1_000);
    join(&mut w.svm, &d);
    try_ix(&mut w.svm, open_round_ix(d.pubkey(), [1u8; 32], 10, 2, sha256(&[1u8; 32])), &d).unwrap();
    at_slot(&mut w.svm, 1_040, &recent(1_040, &[]));
    let round = round_address([1u8; 32]);
    try_ix(&mut w.svm, reveal_ix(round, d.pubkey(), [1u8; 32]), &d).unwrap();
    assert_eq!(seat(&w.svm, 1).since, 1_000);
    let err = commit(&mut w, &d, round, SERVICE).unwrap_err();
    assert!(err.contains("SatDownSince"), "{err}");
}
