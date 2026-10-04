//! Challenges: a member shows the reply behind a reading, or loses their
//! stake. Run against the compiled program.

mod common;
use common::*;
use sasona::{
    canonical_question, verdict_of, Challenge, Reading, ANSWER_WINDOW_SECONDS, CHALLENGE_ANSWERED, CHALLENGE_BOND_LAMPORTS, CHALLENGE_WINDOW_SECONDS,
    CHALLENGE_UPHELD, MAX_REPLY_BYTES, MEMBER_SLASHED, MEMBER_STAKE, READING_FALSE, READING_REVEALED,
};

const SERVICE: &str = "https://sandbox.example.net/run/python";
// The nonce of sasona-protocol's vectors/question.json verdicts, 0.3.0, and
// the reply that delivers for it.
const NONCE: [u8; 16] = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
const ANSWER: &[u8] = b"3eb1bd439947eb76\n";
const CANNED: &[u8] = b"{\"status\":\"ok\",\"output\":\"done\"}";

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
}

fn seconds_pass(svm: &mut LiteSVM, seconds: i64) {
    let mut c: solana_clock::Clock = svm.get_sysvar();
    c.unix_timestamp += seconds;
    svm.set_sysvar(&c);
}

fn lamports(svm: &LiteSVM, at: Address) -> u64 {
    svm.get_account(&at).map(|a| a.lamports).unwrap_or(0)
}

fn coins(svm: &LiteSVM, who: Address) -> u64 {
    token_balance(svm, ata(who, pda(&[COIN_SEED])))
}

/// Membership 1 reads SERVICE and reveals `reply`'s hash with `verdict`, at
/// slot 1,101. Returns the reading.
fn revealed(w: &mut World, reply: &[u8], verdict: u8) -> Address {
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
    w.svm.warp_to_slot(1_101);
    try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), reading, NONCE, sha256(reply), verdict), &d).unwrap();
    reading
}

/// Someone who holds coins, so a tenth of a stake has somewhere to go.
fn challenger(svm: &mut LiteSVM) -> Keypair {
    let (k, usd) = newcomer(svm, 10);
    try_deposit(svm, &k, usd, 10 * DOLLAR).unwrap();
    k
}

#[test]
fn the_program_judges_replies_as_the_protocol_does() {
    // Every reply in sasona-protocol's vectors/question.json, 0.3.0.
    for (reply, verdict) in [
        ("336562316264343339393437656237360a", 1),
        ("7b227374646f7574223a2022336562316264343339393437656237365c6e222c202265786974223a20307d", 1),
        ("ff0033656231626434333939343765623736fe", 1),
        ("3030336562316264343339393437656237366666", 1),
        ("696d706f727420686173686c69620a7072696e7428686173686c69622e73686132353628223031323334353637383961626364656630313233343536373839616263646566222e656e636f64652829292e68657864696765737428295b3a31365d29", 2),
        ("7b22737461747573223a226f6b222c226f7574707574223a22646f6e65227d", 2),
        ("33454231424434333939343745423736", 2),
        ("33656231626434330a3939343765623736", 2),
        ("336562316264343339393437656237", 2),
        ("", 3),
    ] {
        assert_eq!(verdict_of(&unhex(reply), &NONCE), verdict, "{reply}");
    }
}

#[test]
fn a_challenge_answered_with_the_reply_pays_the_member() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let reading = revealed(&mut w, ANSWER, 1);
    let c = challenger(&mut w.svm);
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap();
    let ch: Challenge = read(&w.svm, challenge_address(reading));
    let now = w.svm.get_sysvar::<solana_clock::Clock>().unix_timestamp;
    assert_eq!((addr(ch.challenger), ch.deadline), (c.pubkey(), now + ANSWER_WINDOW_SECONDS));
    assert_eq!(member(&w.svm, 1).open_challenges, 1);

    put_evidence(&mut w.svm, &d, reading, ANSWER).unwrap();
    let before = lamports(&w.svm, d.pubkey());
    let held = lamports(&w.svm, challenge_address(reading));
    try_with(&mut w.svm, |s| answer_ix(s, reading, NONCE), &c).unwrap();
    assert_eq!(lamports(&w.svm, d.pubkey()), before + CHALLENGE_BOND_LAMPORTS);
    assert_eq!(lamports(&w.svm, challenge_address(reading)), held - CHALLENGE_BOND_LAMPORTS);
    let ch: Challenge = read(&w.svm, challenge_address(reading));
    assert_eq!(ch.state, CHALLENGE_ANSWERED);
    assert_eq!(member(&w.svm, 1).open_challenges, 0);
    let r: Reading = read(&w.svm, reading);
    assert_eq!(r.state, READING_REVEALED);

    let err = try_with(&mut w.svm, |s| answer_ix(s, reading, NONCE), &c).unwrap_err();
    assert!(err.contains("ChallengeClosed"), "{err}");
    days_pass(&mut w.svm, 8);
    let err = try_with(&mut w.svm, |s| uphold_ix(s, reading), &c).unwrap_err();
    assert!(err.contains("ChallengeClosed"), "{err}");
}

#[test]
fn a_long_reply_goes_up_in_pieces() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let mut reply = vec![b'x'; MAX_REPLY_BYTES as usize - ANSWER.len()];
    reply.extend_from_slice(ANSWER);
    let reading = revealed(&mut w, &reply, 1);
    let c = challenger(&mut w.svm);
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap();
    put_evidence(&mut w.svm, &d, reading, &reply).unwrap();
    try_with(&mut w.svm, |s| answer_ix(s, reading, NONCE), &c).unwrap();
}

#[test]
fn a_reply_longer_than_the_limit_cannot_go_up() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let reading = revealed(&mut w, ANSWER, 1);
    let err = try_ix(&mut w.svm, open_evidence_ix(d.pubkey(), reading, MAX_REPLY_BYTES + 1), &d).unwrap_err();
    assert!(err.contains("ReplyTooLong"), "{err}");
    try_ix(&mut w.svm, open_evidence_ix(d.pubkey(), reading, 4), &d).unwrap();
    let err = try_ix(&mut w.svm, write_evidence_ix(d.pubkey(), reading, 2, b"abc"), &d).unwrap_err();
    assert!(err.contains("ReplyTooLong"), "{err}");
}

#[test]
fn only_the_reader_puts_a_reply_on_chain() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let reading = revealed(&mut w, ANSWER, 1);
    let (stranger, _) = newcomer(&mut w.svm, 0);
    let err = try_ix(&mut w.svm, open_evidence_ix(stranger.pubkey(), reading, 17), &stranger).unwrap_err();
    assert!(err.contains("NotTheReader"), "{err}");
    try_ix(&mut w.svm, open_evidence_ix(d.pubkey(), reading, 17), &d).unwrap();
    let err = try_ix(&mut w.svm, write_evidence_ix(stranger.pubkey(), reading, 0, ANSWER), &stranger).unwrap_err();
    assert!(err.contains("NotTheReader"), "{err}");
}

#[test]
fn a_reply_that_is_not_the_one_recorded_does_not_answer() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let reading = revealed(&mut w, ANSWER, 1);
    let c = challenger(&mut w.svm);
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap();
    put_evidence(&mut w.svm, &d, reading, b"3eb1bd439947eb76 and more").unwrap();
    let err = try_with(&mut w.svm, |s| answer_ix(s, reading, NONCE), &c).unwrap_err();
    assert!(err.contains("NotTheReply"), "{err}");
}

#[test]
fn a_verdict_that_does_not_follow_from_the_reply_does_not_answer() {
    // The reading says delivered, and the reply it recorded is canned.
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let reading = revealed(&mut w, CANNED, 1);
    let c = challenger(&mut w.svm);
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap();
    put_evidence(&mut w.svm, &d, reading, CANNED).unwrap();
    let err = try_with(&mut w.svm, |s| answer_ix(s, reading, NONCE), &c).unwrap_err();
    assert!(err.contains("VerdictDoesNotFollow"), "{err}");
}

#[test]
fn the_answer_needs_the_readings_own_nonce() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let reading = revealed(&mut w, ANSWER, 1);
    let c = challenger(&mut w.svm);
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap();
    put_evidence(&mut w.svm, &d, reading, ANSWER).unwrap();
    let err = try_with(&mut w.svm, |s| answer_ix(s, reading, [9u8; 16]), &c).unwrap_err();
    assert!(err.contains("AccountNotInitialized"), "{err}");
}

#[test]
fn an_answer_after_the_deadline_is_refused() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let reading = revealed(&mut w, ANSWER, 1);
    let c = challenger(&mut w.svm);
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap();
    put_evidence(&mut w.svm, &d, reading, ANSWER).unwrap();
    days_pass(&mut w.svm, 8);
    let err = try_with(&mut w.svm, |s| answer_ix(s, reading, NONCE), &c).unwrap_err();
    assert!(err.contains("WindowClosed"), "{err}");
}

#[test]
fn an_unanswered_challenge_takes_the_stake() {
    let mut w = world_with_member();
    let reading = revealed(&mut w, CANNED, 1);
    let c = challenger(&mut w.svm);
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap();
    seconds_pass(&mut w.svm, ANSWER_WINDOW_SECONDS);
    let err = try_with(&mut w.svm, |s| uphold_ix(s, reading), &c).unwrap_err();
    assert!(err.contains("NotLapsedYet"), "{err}");

    seconds_pass(&mut w.svm, 1);
    let (anyone, _) = newcomer(&mut w.svm, 0);
    let coins_before = coins(&w.svm, c.pubkey());
    let sol_before = lamports(&w.svm, c.pubkey());
    try_with(&mut w.svm, |s| uphold_ix(s, reading), &anyone).unwrap();
    assert_eq!(coins(&w.svm, c.pubkey()), coins_before + MEMBER_STAKE / 10);
    assert_eq!(lamports(&w.svm, c.pubkey()), sol_before + CHALLENGE_BOND_LAMPORTS);
    assert_eq!(token_balance(&w.svm, pda(&[sasona::HELD_SEED])), MEMBER_STAKE - MEMBER_STAKE / 10);
    assert_eq!(token_balance(&w.svm, pda(&[sasona::STAKES_SEED])), 0);
    let m = member(&w.svm, 1);
    assert_eq!((m.state, m.stake, m.open_challenges), (MEMBER_SLASHED, 0, 0));
    let r: Reading = read(&w.svm, reading);
    assert_eq!(r.state, READING_FALSE);
    let ch: Challenge = read(&w.svm, challenge_address(reading));
    assert_eq!(ch.state, CHALLENGE_UPHELD);
    let err = try_with(&mut w.svm, |s| uphold_ix(s, reading), &anyone).unwrap_err();
    assert!(err.contains("ChallengeClosed"), "{err}");
}

#[test]
fn a_member_who_lost_their_stake_cannot_read_or_leave() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let reading = revealed(&mut w, CANNED, 1);
    let c = challenger(&mut w.svm);
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap();
    days_pass(&mut w.svm, 8);
    try_with(&mut w.svm, |s| uphold_ix(s, reading), &c).unwrap();

    let err = try_with(&mut w.svm, |s| ask_to_leave_ix(s, d.pubkey(), 1), &d).unwrap_err();
    assert!(err.contains("NotActive"), "{err}");
    let err = try_ix(&mut w.svm, leave_ix(d.pubkey(), 1), &d).unwrap_err();
    assert!(err.contains("NotLeaving"), "{err}");

    let slot = 2_000;
    w.svm.warp_to_slot(slot);
    try_ix(&mut w.svm, open_round_ix(d.pubkey(), [2u8; 32], 10, 2, sha256(&[2u8; 32])), &d).unwrap();
    at_slot(&mut w.svm, slot + 40, &recent(slot + 40, &[]));
    try_ix(&mut w.svm, reveal_ix(round_address([2u8; 32]), d.pubkey(), [2u8; 32]), &d).unwrap();
    let q = sha256(&canonical_question(&[3u8; 16]));
    let err = try_with(&mut w.svm, |s| commit_reading_ix(s, d.pubkey(), round_address([2u8; 32]), SERVICE, q), &d).unwrap_err();
    assert!(err.contains("NotDrawn"), "{err}");
}

#[test]
fn a_false_reading_cannot_be_re_tested() {
    let mut w = world_with_member();
    let reading = revealed(&mut w, CANNED, 1);
    let c = challenger(&mut w.svm);
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap();
    let slot = 2_000;
    days_pass(&mut w.svm, 8);
    try_with(&mut w.svm, |s| uphold_ix(s, reading), &c).unwrap();

    let (b, _) = new_member(&mut w.svm);
    w.svm.warp_to_slot(slot + 10);
    try_ix(&mut w.svm, open_round_ix(b.pubkey(), [2u8; 32], 10, 2, sha256(&[2u8; 32])), &b).unwrap();
    at_slot(&mut w.svm, slot + 50, &recent(slot + 50, &[]));
    try_ix(&mut w.svm, reveal_ix(round_address([2u8; 32]), b.pubkey(), [2u8; 32]), &b).unwrap();
    let q = sha256(&canonical_question(&[3u8; 16]));
    let err = try_with(&mut w.svm, |s| commit_second_ix(s, b.pubkey(), round_address([2u8; 32]), SERVICE, q, reading), &b)
        .unwrap_err();
    assert!(err.contains("FirstNotRevealed"), "{err}");
}

#[test]
fn a_member_answers_within_their_notice_and_then_leaves() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let reading = revealed(&mut w, ANSWER, 1);
    let c = challenger(&mut w.svm);
    try_with(&mut w.svm, |s| ask_to_leave_ix(s, d.pubkey(), 1), &d).unwrap();
    days_pass(&mut w.svm, 29);
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap();
    days_pass(&mut w.svm, 6);
    put_evidence(&mut w.svm, &d, reading, ANSWER).unwrap();
    try_with(&mut w.svm, |s| answer_ix(s, reading, NONCE), &c).unwrap();
    days_pass(&mut w.svm, 10);
    try_ix(&mut w.svm, leave_ix(d.pubkey(), 1), &d).unwrap();
}

#[test]
fn a_challenge_nobody_has_upheld_yet_still_holds_the_stake() {
    // The answer's time ran out, but nobody has upheld it: the member still
    // cannot take the stake and run.
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let reading = revealed(&mut w, CANNED, 1);
    let c = challenger(&mut w.svm);
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap();
    try_with(&mut w.svm, |s| ask_to_leave_ix(s, d.pubkey(), 1), &d).unwrap();
    days_pass(&mut w.svm, 46);
    let err = try_ix(&mut w.svm, leave_ix(d.pubkey(), 1), &d).unwrap_err();
    assert!(err.contains("ChallengeOpen"), "{err}");
    let coins_before = coins(&w.svm, c.pubkey());
    try_with(&mut w.svm, |s| uphold_ix(s, reading), &c).unwrap();
    assert_eq!(coins(&w.svm, c.pubkey()), coins_before + MEMBER_STAKE / 10);
    assert_eq!(member(&w.svm, 1).state, MEMBER_SLASHED);
    let err = try_ix(&mut w.svm, leave_ix(d.pubkey(), 1), &d).unwrap_err();
    assert!(err.contains("NotLeaving"), "{err}");
}

#[test]
fn a_reading_can_be_challenged_for_thirty_days() {
    let mut w = world_with_member();
    let reading = revealed(&mut w, ANSWER, 1);
    let c = challenger(&mut w.svm);
    seconds_pass(&mut w.svm, CHALLENGE_WINDOW_SECONDS + 1);
    let err = try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap_err();
    assert!(err.contains("WindowClosed"), "{err}");
}

#[test]
fn a_reading_can_be_challenged_on_its_thirtieth_day() {
    let mut w = world_with_member();
    let reading = revealed(&mut w, ANSWER, 1);
    let c = challenger(&mut w.svm);
    seconds_pass(&mut w.svm, CHALLENGE_WINDOW_SECONDS);
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap();
}

#[test]
fn an_answered_reply_can_no_longer_change() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let reading = revealed(&mut w, ANSWER, 1);
    let c = challenger(&mut w.svm);
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap();
    put_evidence(&mut w.svm, &d, reading, ANSWER).unwrap();
    try_ix(&mut w.svm, write_evidence_ix(d.pubkey(), reading, 0, b"x"), &d).unwrap();
    let err = try_with(&mut w.svm, |s| answer_ix(s, reading, NONCE), &c).unwrap_err();
    assert!(err.contains("NotTheReply"), "{err}");
    try_ix(&mut w.svm, write_evidence_ix(d.pubkey(), reading, 0, b"3"), &d).unwrap();
    try_with(&mut w.svm, |s| answer_ix(s, reading, NONCE), &c).unwrap();
    let err = try_ix(&mut w.svm, write_evidence_ix(d.pubkey(), reading, 0, b"x"), &d).unwrap_err();
    assert!(err.contains("EvidenceSealed"), "{err}");
    let e: sasona::Evidence = read(&w.svm, evidence_address(reading));
    assert_eq!((e.sealed, e.reply.as_slice()), (true, ANSWER));
}

#[test]
fn a_member_who_asked_to_leave_still_loses_the_stake() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let reading = revealed(&mut w, CANNED, 1);
    let c = challenger(&mut w.svm);
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap();
    try_with(&mut w.svm, |s| ask_to_leave_ix(s, d.pubkey(), 1), &d).unwrap();
    days_pass(&mut w.svm, 8);
    try_with(&mut w.svm, |s| uphold_ix(s, reading), &c).unwrap();
    assert_eq!(token_balance(&w.svm, pda(&[sasona::HELD_SEED])), MEMBER_STAKE - MEMBER_STAKE / 10);
    let m = member(&w.svm, 1);
    assert_eq!((m.state, m.stake, m.seat), (MEMBER_SLASHED, 0, 0));
}

#[test]
fn a_member_who_lost_their_seat_hands_it_to_the_last() {
    let mut w = world_with_member();
    let reading = revealed(&mut w, CANNED, 1);
    let (b, _) = new_member(&mut w.svm);
    let c = challenger(&mut w.svm);
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap();
    days_pass(&mut w.svm, 8);
    try_with(&mut w.svm, |s| uphold_ix(s, reading), &c).unwrap();
    assert_eq!(seated(&w.svm), 1);
    let s1 = seat(&w.svm, 1);
    assert_eq!((s1.member, addr(s1.owner)), (2, b.pubkey()));
    assert_eq!(member(&w.svm, 2).seat, 1);
    // Only the stake taken left the vault; b's is still there.
    assert_eq!(token_balance(&w.svm, pda(&[sasona::STAKES_SEED])), MEMBER_STAKE);
}

#[test]
fn a_second_upheld_challenge_finds_no_stake_left() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let one = revealed(&mut w, CANNED, 1);
    let round = round_address([1u8; 32]);
    let other_service = "https://data.example.com/prices";
    let q = sha256(&canonical_question(&[5u8; 16]));
    try_with(&mut w.svm, |s| commit_reading_ix(s, d.pubkey(), round, other_service, q), &d).unwrap();
    let two = reading_address(round, other_service);
    w.svm.warp_to_slot(1_102);
    try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), two, [5u8; 16], sha256(CANNED), 1), &d).unwrap();

    let c = challenger(&mut w.svm);
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), one), &c).unwrap();
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), two), &c).unwrap();
    assert_eq!(member(&w.svm, 1).open_challenges, 2);
    days_pass(&mut w.svm, 8);
    try_with(&mut w.svm, |s| uphold_ix(s, one), &c).unwrap();
    let coins_before = coins(&w.svm, c.pubkey());
    try_with(&mut w.svm, |s| uphold_ix(s, two), &c).unwrap();
    assert_eq!(coins(&w.svm, c.pubkey()), coins_before);
    assert_eq!(token_balance(&w.svm, pda(&[sasona::HELD_SEED])), MEMBER_STAKE - MEMBER_STAKE / 10);
    let m = member(&w.svm, 1);
    assert_eq!((m.state, m.open_challenges), (MEMBER_SLASHED, 0));
    let r: Reading = read(&w.svm, two);
    assert_eq!(r.state, READING_FALSE);
}

#[test]
fn only_a_revealed_reading_can_be_challenged_and_only_once() {
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let seed = [1u8; 32];
    w.svm.warp_to_slot(1_000);
    try_ix(&mut w.svm, open_round_ix(d.pubkey(), [1u8; 32], 10, 2, sha256(&seed)), &d).unwrap();
    at_slot(&mut w.svm, 1_040, &recent(1_040, &[]));
    let round = round_address([1u8; 32]);
    try_ix(&mut w.svm, reveal_ix(round, d.pubkey(), seed), &d).unwrap();
    let q = sha256(&canonical_question(&NONCE));
    try_with(&mut w.svm, |s| commit_reading_ix(s, d.pubkey(), round, SERVICE, q), &d).unwrap();
    let reading = reading_address(round, SERVICE);
    let c = challenger(&mut w.svm);
    let err = try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap_err();
    assert!(err.contains("ReadingNotOpen"), "{err}");

    w.svm.warp_to_slot(1_041);
    try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), reading, NONCE, sha256(ANSWER), 1), &d).unwrap();
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap();
    let err = try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap_err();
    assert!(err.contains("already in use"), "{err}");
}

#[test]
fn the_answer_cannot_borrow_another_readings_nonce() {
    // Another reading's nonce, whose record exists, would let a member pick
    // whichever nonce makes their reply pass.
    let mut w = world_with_member();
    let d = w.depositor.insecure_clone();
    let reading = revealed(&mut w, ANSWER, 1);
    let round = round_address([1u8; 32]);
    let other_service = "https://data.example.com/prices";
    let q = sha256(&canonical_question(&[5u8; 16]));
    try_with(&mut w.svm, |s| commit_reading_ix(s, d.pubkey(), round, other_service, q), &d).unwrap();
    let other = reading_address(round, other_service);
    w.svm.warp_to_slot(1_102);
    try_ix(&mut w.svm, reveal_reading_ix(d.pubkey(), other, [5u8; 16], [0u8; 32], 2), &d).unwrap();

    let c = challenger(&mut w.svm);
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), reading), &c).unwrap();
    put_evidence(&mut w.svm, &d, reading, ANSWER).unwrap();
    let err = try_with(&mut w.svm, |s| answer_ix(s, reading, [5u8; 16]), &c).unwrap_err();
    assert!(err.contains("NonceTaken"), "{err}");
    try_with(&mut w.svm, |s| answer_ix(s, reading, NONCE), &c).unwrap();
}

#[test]
fn a_pair_whose_first_reading_was_upheld_false_does_not_settle() {
    let mut w = world_with_member();
    let first = revealed(&mut w, CANNED, 1);
    let (b, _) = new_member(&mut w.svm);
    w.svm.warp_to_slot(1_200);
    let seed = [2u8; 32];
    try_ix(&mut w.svm, open_round_ix(b.pubkey(), [2u8; 32], 10, 2, sha256(&seed)), &b).unwrap();
    at_slot(&mut w.svm, 1_240, &recent(1_240, &[]));
    let reread = round_address([2u8; 32]);
    try_ix(&mut w.svm, reveal_ix(reread, b.pubkey(), seed), &b).unwrap();
    let q = sha256(&canonical_question(&[6u8; 16]));
    try_with(&mut w.svm, |s| commit_second_ix(s, b.pubkey(), reread, SERVICE, q, first), &b).unwrap();
    let second = reading_address(reread, SERVICE);
    w.svm.warp_to_slot(1_241);
    try_ix(&mut w.svm, reveal_reading_ix(b.pubkey(), second, [6u8; 16], [9u8; 32], 2), &b).unwrap();

    let c = challenger(&mut w.svm);
    try_with(&mut w.svm, |s| challenge_ix(s, c.pubkey(), first), &c).unwrap();
    days_pass(&mut w.svm, 8);
    try_with(&mut w.svm, |s| uphold_ix(s, first), &c).unwrap();
    let err = try_ix(&mut w.svm, settle_pair_ix(first, second), &c).unwrap_err();
    assert!(err.contains("FirstNotRevealed"), "{err}");
}
