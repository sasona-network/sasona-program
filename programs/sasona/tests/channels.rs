//! Payment channels (sasona-protocol SPEC.md section 8), against the
//! compiled program. The amounts are those of vectors/channel.json.

mod common;
use common::*;

const PUT_IN: u64 = DOLLAR;

struct Ch {
    w: World,
    payer: Keypair,
    payee: Keypair,
    ch: Address,
}

/// A payer with $10 opens channel 0 to a payee, with $1 in it, signing its
/// own vouchers.
fn setup() -> Ch {
    let mut w = world_with_member();
    let payer = new_buyer(&mut w.svm, 10 * DOLLAR);
    let payee = Keypair::new();
    w.svm.airdrop(&payee.pubkey(), 10_000_000_000).unwrap();
    usd_of(&mut w.svm, payee.pubkey());
    try_ix(&mut w.svm, open_channel_ix(payer.pubkey(), payer.pubkey(), payee.pubkey(), 0, PUT_IN), &payer).unwrap();
    let ch = channel_address(payer.pubkey(), payee.pubkey(), 0);
    Ch { w, payer, payee, ch }
}

fn pay_with(c: &mut Ch, voucher: Instruction, amount: u64) -> Result<(), String> {
    let take = take_payment_ix(&c.w.svm, c.ch, amount);
    let payee = c.payee.insecure_clone();
    send_all(&mut c.w.svm, &[voucher, take], &[&payee])
}

fn pay(c: &mut Ch, amount: u64) -> Result<(), String> {
    let v = voucher_ix(&c.payer, c.ch, amount);
    pay_with(c, v, amount)
}

fn payee_usd(c: &Ch) -> u64 {
    token_balance(&c.w.svm, ata(c.payee.pubkey(), usd()))
}

fn fees(c: &Ch) -> u64 {
    token_balance(&c.w.svm, pda(&[FEES_SEED]))
}

fn sweep(c: &mut Ch) -> Result<(), String> {
    let payee = c.payee.insecure_clone();
    try_ix(&mut c.w.svm, sweep_channel_ix(payee.pubkey(), c.ch), &payee)
}

fn close_by(c: &mut Ch, who: &Keypair) -> Result<(), String> {
    let ix = close_channel_ix(&c.w.svm, who.pubkey(), c.ch);
    try_ix(&mut c.w.svm, ix, who)
}

#[test]
fn a_channel_through_its_life_as_the_vectors_have_it() {
    let mut c = setup();
    let fees_before = fees(&c);
    pay(&mut c, 100_000).unwrap();
    assert_eq!(payee_usd(&c), 100_000);
    let k = channel(&c.w.svm, c.ch);
    assert_eq!((k.taken, k.charged, k.owed), (100_000, 15_000, 15_000));

    pay(&mut c, 100_007).unwrap();
    assert_eq!(payee_usd(&c), 100_007, "7 more");
    let k = channel(&c.w.svm, c.ch);
    assert_eq!((k.charged, k.owed), (15_002, 15_002), "the markup on the total, less what was charged");

    let err = pay(&mut c, 100_007).unwrap_err();
    assert!(err.contains("NothingMoreToPay"), "{err}");

    sweep(&mut c).unwrap();
    assert_eq!(fees(&c), fees_before + 15_002);
    assert_eq!(markup_held(&c.w.svm), 15_002);
    assert_eq!(channel(&c.w.svm, c.ch).owed, 0);
    let err = sweep(&mut c).unwrap_err();
    assert!(err.contains("NothingOwed"), "{err}");

    // More than the channel can pay: it pays what it can.
    pay(&mut c, 900_000).unwrap();
    assert_eq!(payee_usd(&c), 869_565);
    let k = channel(&c.w.svm, c.ch);
    assert_eq!((k.taken, k.charged, k.owed), (869_565, 130_435, 115_433));
    assert_eq!(k.taken + k.charged, PUT_IN);

    // Someone sends 10 units to the channel's account.
    let at = channel_usd_address(c.ch);
    let held = token_balance(&c.w.svm, at);
    put_token_account(&mut c.w.svm, at, usd(), c.ch, held + 10);

    let payer_usd_before = usd_balance(&c.w.svm, c.payer.pubkey());
    let payer_sol_before = c.w.svm.get_account(&c.payer.pubkey()).unwrap().lamports;
    let payee = c.payee.insecure_clone();
    close_by(&mut c, &payee).unwrap();
    assert_eq!(fees(&c), fees_before + 130_435, "all the markup reached the network");
    assert_eq!(markup_held(&c.w.svm), 130_435);
    assert_eq!(usd_balance(&c.w.svm, c.payer.pubkey()), payer_usd_before + 10, "the excess goes back to the payer");
    assert!(c.w.svm.get_account(&c.ch).is_none_or(|a| a.lamports == 0));
    assert!(c.w.svm.get_account(&at).is_none_or(|a| a.lamports == 0));
    assert!(c.w.svm.get_account(&c.payer.pubkey()).unwrap().lamports > payer_sol_before, "both rents back to the payer");
}

fn usd_balance(svm: &LiteSVM, who: Address) -> u64 {
    token_balance(svm, ata(who, usd()))
}

#[test]
fn identifiers_only_go_up_so_old_vouchers_never_come_back() {
    let mut c = setup();
    pay(&mut c, 100_000).unwrap();
    let old = voucher_ix(&c.payer, c.ch, 100_000);
    let payee = c.payee.insecure_clone();
    close_by(&mut c, &payee).unwrap();
    // The same address again: refused, so the old voucher stays dead.
    let payer = c.payer.insecure_clone();
    let err = try_ix(&mut c.w.svm, open_channel_ix(payer.pubkey(), payer.pubkey(), payee.pubkey(), 0, PUT_IN), &payer).unwrap_err();
    assert!(err.contains("NotTheNextId"), "{err}");
    let err = try_ix(&mut c.w.svm, open_channel_ix(payer.pubkey(), payer.pubkey(), payee.pubkey(), 2, PUT_IN), &payer).unwrap_err();
    assert!(err.contains("NotTheNextId"), "{err}");
    try_ix(&mut c.w.svm, open_channel_ix(payer.pubkey(), payer.pubkey(), payee.pubkey(), 1, PUT_IN), &payer).unwrap();
    // The old voucher names the old channel's address, so it is worth nothing here.
    c.ch = channel_address(payer.pubkey(), payee.pubkey(), 1);
    let err = pay_with(&mut c, old, 100_000).unwrap_err();
    assert!(err.contains("BadVoucher"), "{err}");
}

#[test]
fn the_signer_signs_the_opening_and_is_not_the_payee() {
    let mut w = world_with_member();
    let payer = new_buyer(&mut w.svm, 10 * DOLLAR);
    let payee = Keypair::new();
    let other = Keypair::new();
    // A signer that did not sign the opening.
    let mut ix = open_channel_ix(payer.pubkey(), other.pubkey(), payee.pubkey(), 0, PUT_IN);
    ix.accounts[1].is_signer = false;
    let err = try_ix(&mut w.svm, ix, &payer).unwrap_err();
    assert!(err.contains("AccountNotSigner"), "{err}");
    // The payee's own key as signer: it could sign its own vouchers.
    let err = send_all(&mut w.svm, &[open_channel_ix(payer.pubkey(), payee.pubkey(), payee.pubkey(), 0, PUT_IN)], &[&payer, &payee])
        .unwrap_err();
    assert!(err.contains("SignerIsPayee"), "{err}");
    // The pool as payee: what it got would be counted by nobody.
    let pool = pda(&[POOL_SEED]);
    let err = try_ix(&mut w.svm, open_channel_ix(payer.pubkey(), payer.pubkey(), pool, 0, PUT_IN), &payer).unwrap_err();
    assert!(err.contains("NotAPayee"), "{err}");
    let network = pda(&[NETWORK_SEED]);
    let err = try_ix(&mut w.svm, open_channel_ix(payer.pubkey(), payer.pubkey(), network, 0, PUT_IN), &payer).unwrap_err();
    assert!(err.contains("NotAPayee"), "{err}");
}

#[test]
fn a_channel_honours_only_its_signers_vouchers() {
    let mut w = world_with_member();
    let payer = new_buyer(&mut w.svm, 10 * DOLLAR);
    let payee = Keypair::new();
    w.svm.airdrop(&payee.pubkey(), 10_000_000_000).unwrap();
    usd_of(&mut w.svm, payee.pubkey());
    let signer = Keypair::new();
    send_all(&mut w.svm, &[open_channel_ix(payer.pubkey(), signer.pubkey(), payee.pubkey(), 0, PUT_IN)], &[&payer, &signer]).unwrap();
    let ch = channel_address(payer.pubkey(), payee.pubkey(), 0);
    let mut c = Ch { w, payer, payee, ch };
    // The payer's own key is not the channel's signer.
    let err = pay(&mut c, 1_000).unwrap_err();
    assert!(err.contains("BadVoucher"), "{err}");
    let v = voucher_ix(&signer, ch, 1_000);
    pay_with(&mut c, v, 1_000).unwrap();
    assert_eq!(payee_usd(&c), 1_000);
}

#[test]
fn a_voucher_must_be_signed_for_this_amount_channel_and_cluster() {
    let mut c = setup();
    let none = take_payment_ix(&c.w.svm, c.ch, 1_000);
    let payee = c.payee.insecure_clone();
    let err = send_all(&mut c.w.svm, &[none], &[&payee]).unwrap_err();
    assert!(err.contains("NoVoucher"), "no signature at all: {err}");

    let stranger = Keypair::new();
    let v = voucher_ix(&stranger, c.ch, 1_000);
    let err = pay_with(&mut c, v, 1_000).unwrap_err();
    assert!(err.contains("BadVoucher"), "another key: {err}");

    let v = voucher_ix(&c.payer, c.ch, 100);
    let err = pay_with(&mut c, v, 1_000).unwrap_err();
    assert!(err.contains("BadVoucher"), "signed for less: {err}");

    let other_channel = channel_address(c.payer.pubkey(), stranger.pubkey(), 0);
    let v = voucher_ix(&c.payer, other_channel, 1_000);
    let err = pay_with(&mut c, v, 1_000).unwrap_err();
    assert!(err.contains("BadVoucher"), "signed for another channel: {err}");

    let mut message = sasona::voucher_message(&key(c.ch), 1_000);
    message[49] = 2;
    let sig = c.payer.sign_message(&message);
    let v = ed25519_ix(c.payer.pubkey(), sig.as_ref(), &message);
    let err = pay_with(&mut c, v, 1_000).unwrap_err();
    assert!(err.contains("BadVoucher"), "signed for another cluster: {err}");

    pay(&mut c, 1_000).unwrap();
    assert_eq!(payee_usd(&c), 1_000);
}

#[test]
fn the_signature_check_cannot_be_pointed_elsewhere() {
    let mut c = setup();
    let message = sasona::voucher_message(&key(c.ch), 1_000);
    let sig = c.payer.sign_message(&message);

    // Index fields that are not u16::MAX, even pointing at the instruction
    // itself, which the ed25519 program accepts.
    let mut tail = c.payer.pubkey().to_bytes().to_vec();
    tail.extend(sig.as_ref());
    tail.extend(&message);
    let v = ed25519_raw(1, [48, 0, 16, 0, 112, 90, 0], &tail);
    let err = pay_with(&mut c, v, 1_000).unwrap_err();
    assert!(err.contains("BadVoucher"), "index fields: {err}");

    // The payer's key sitting where a careless reader looks, while the
    // signature actually checked is a stranger's, over the same message.
    let stranger = Keypair::new();
    let theirs = stranger.sign_message(&message);
    let mut tail = c.payer.pubkey().to_bytes().to_vec();
    tail.extend(stranger.pubkey().to_bytes());
    tail.extend(theirs.as_ref());
    tail.extend(&message);
    let v = ed25519_raw(1, [80, u16::MAX, 48, u16::MAX, 144, 90, u16::MAX], &tail);
    let err = pay_with(&mut c, v, 1_000).unwrap_err();
    assert!(err.contains("BadVoucher"), "moved offsets: {err}");

    // Two signatures in one check, the payer's and a stranger's.
    let mut d = vec![2u8, 0];
    // Two 14-byte headers after the first two bytes: the data starts at 30.
    for f in [62u16, u16::MAX, 30, u16::MAX, 222, 90, u16::MAX, 158, u16::MAX, 126, u16::MAX, 222, 90, u16::MAX] {
        d.extend(f.to_le_bytes());
    }
    d.extend(c.payer.pubkey().to_bytes());
    d.extend(sig.as_ref());
    d.extend(stranger.pubkey().to_bytes());
    d.extend(theirs.as_ref());
    d.extend(&message);
    let v = Instruction { program_id: addr(ED25519), accounts: vec![], data: d };
    let err = pay_with(&mut c, v, 1_000).unwrap_err();
    assert!(err.contains("BadVoucher"), "two signatures: {err}");

    // An instruction between the signature and the payment.
    let payee = c.payee.insecure_clone();
    let between = Instruction {
        program_id: addr(anchor_lang::system_program::ID),
        accounts: vec![AccountMeta::new(payee.pubkey(), true), AccountMeta::new(payee.pubkey(), false)],
        data: [2u32.to_le_bytes().as_slice(), 0u64.to_le_bytes().as_slice()].concat(),
    };
    let v = voucher_ix(&c.payer, c.ch, 1_000);
    let take = take_payment_ix(&c.w.svm, c.ch, 1_000);
    let err = send_all(&mut c.w.svm, &[v, between, take], &[&payee]).unwrap_err();
    assert!(err.contains("NoVoucher"), "not just before: {err}");

    // A signature that does not verify fails the whole transaction.
    let mut bad = sig.as_ref().to_vec();
    bad[0] ^= 1;
    let v = ed25519_ix(c.payer.pubkey(), &bad, &message);
    assert!(pay_with(&mut c, v, 1_000).is_err());
    assert_eq!(payee_usd(&c), 0);
}

#[test]
fn only_the_payees_own_account_is_paid() {
    let mut c = setup();
    let v = voucher_ix(&c.payer, c.ch, 1_000);
    let mut take = take_payment_ix(&c.w.svm, c.ch, 1_000);
    let thief = Keypair::new();
    take.accounts[2].pubkey = usd_of(&mut c.w.svm, thief.pubkey());
    let payee = c.payee.insecure_clone();
    let err = send_all(&mut c.w.svm, &[v, take], &[&payee]).unwrap_err();
    assert!(err.contains("NotThePayee"), "{err}");
}

#[test]
fn anyone_may_show_a_voucher_and_the_money_still_reaches_the_payee() {
    let mut c = setup();
    let relayer = Keypair::new();
    c.w.svm.airdrop(&relayer.pubkey(), 1_000_000_000).unwrap();
    let v = voucher_ix(&c.payer, c.ch, 1_000);
    let take = take_payment_ix(&c.w.svm, c.ch, 1_000);
    send_all(&mut c.w.svm, &[v, take], &[&relayer]).unwrap();
    assert_eq!(payee_usd(&c), 1_000);
}

#[test]
fn the_notice_to_close_is_counted_in_slots() {
    let mut c = setup();
    let payer = c.payer.insecure_clone();
    let stranger = Keypair::new();
    c.w.svm.airdrop(&stranger.pubkey(), 1_000_000_000).unwrap();
    let err = close_by(&mut c, &stranger).unwrap_err();
    assert!(err.contains("NoticeNotOver"), "nobody asked: {err}");

    let payee = c.payee.insecure_clone();
    let err = try_ix(&mut c.w.svm, ask_to_close_channel_ix(payee.pubkey(), c.ch), &payee).unwrap_err();
    assert!(err.contains("NotTheOwner"), "only the payer asks: {err}");
    try_ix(&mut c.w.svm, ask_to_close_channel_ix(payer.pubkey(), c.ch), &payer).unwrap();
    let asked = channel(&c.w.svm, c.ch).asked;
    assert!(asked > 0);
    let err = try_ix(&mut c.w.svm, ask_to_close_channel_ix(payer.pubkey(), c.ch), &payer).unwrap_err();
    assert!(err.contains("AlreadyAsked"), "{err}");
    let err = try_ix(&mut c.w.svm, add_to_channel_ix(payer.pubkey(), c.ch, 1), &payer).unwrap_err();
    assert!(err.contains("ClosePending"), "{err}");

    let end = asked + sasona::NOTICE_SLOTS;
    c.w.svm.warp_to_slot(end - 1);
    pay(&mut c, 1_000).unwrap();
    let err = close_by(&mut c, &stranger).unwrap_err();
    assert!(err.contains("NoticeNotOver"), "{err}");
    c.w.svm.warp_to_slot(end);
    let err = pay(&mut c, 2_000).unwrap_err();
    assert!(err.contains("NoticeOver"), "{err}");
    close_by(&mut c, &stranger).unwrap();
    assert_eq!(usd_balance(&c.w.svm, payer.pubkey()), 10 * DOLLAR - 1_000 - 150);
}

#[test]
fn only_the_payer_adds_and_only_while_open() {
    let mut c = setup();
    let payee = c.payee.insecure_clone();
    usd_of(&mut c.w.svm, payee.pubkey());
    let err = try_ix(&mut c.w.svm, add_to_channel_ix(payee.pubkey(), c.ch, 1), &payee).unwrap_err();
    assert!(err.contains("NotTheOwner") || err.contains("ConstraintTokenOwner"), "{err}");
    let payer = c.payer.insecure_clone();
    try_ix(&mut c.w.svm, add_to_channel_ix(payer.pubkey(), c.ch, DOLLAR), &payer).unwrap();
    assert_eq!(channel(&c.w.svm, c.ch).put_in, 2 * DOLLAR);
    pay(&mut c, 1_500_000).unwrap();
    assert_eq!(payee_usd(&c), 1_500_000);
}

#[test]
fn a_huge_voucher_pays_no_more_than_was_put_in() {
    let mut c = setup();
    pay(&mut c, u64::MAX).unwrap();
    let k = channel(&c.w.svm, c.ch);
    assert_eq!((k.taken, k.charged), (869_565, 130_435));
    assert!(k.taken + k.charged <= PUT_IN);
    let err = pay(&mut c, u64::MAX).unwrap_err();
    assert!(err.contains("NothingMoreToPay"), "{err}");
}

#[test]
fn the_markup_turns_into_coin_with_its_reserve() {
    let mut c = setup();
    pay(&mut c, 600_000).unwrap();
    sweep(&mut c).unwrap();
    let held = markup_held(&c.w.svm);
    assert_eq!(held, 90_000);
    let before = pool(&c.w.svm);
    let fees_before = fees(&c);
    let caller = c.payee.insecure_clone();
    try_ix(&mut c.w.svm, settle_markup_ix(caller.pubkey()), &caller).unwrap();
    let after = pool(&c.w.svm);
    let f = Fee::of(held).unwrap();
    assert_eq!(f.reserve, 30_000, "5 of the 15 points");
    assert_eq!(after.usd_reserve, before.usd_reserve + held, "the reserve stays, and the rest buys coin from the pool");
    assert_eq!(fees(&c), fees_before - held);
    assert_eq!(markup_held(&c.w.svm), 0);
    let err = try_ix(&mut c.w.svm, settle_markup_ix(caller.pubkey()), &caller).unwrap_err();
    assert!(err.contains("NothingDeposited"), "{err}");
    assert_books_balance(&c.w.svm);
}

#[test]
fn a_signature_check_that_is_too_short_or_points_outside_is_refused() {
    let mut c = setup();
    let message = sasona::voucher_message(&key(c.ch), 1_000);
    let sig = c.payer.sign_message(&message);
    let mut tail = c.payer.pubkey().to_bytes().to_vec();
    tail.extend(sig.as_ref());
    tail.extend(&message);

    // A message length other than 90. All 90 bytes are there, but the
    // ed25519 program is told to check 89: the signature, over the first 89,
    // would leave the amount's last byte unsigned.
    let short = c.payer.sign_message(&message[..89]);
    let mut t89 = c.payer.pubkey().to_bytes().to_vec();
    t89.extend(short.as_ref());
    t89.extend(&message);
    let v = ed25519_raw(1, [48, u16::MAX, 16, u16::MAX, 112, 89, u16::MAX], &t89);
    let err = pay_with(&mut c, v, 1_000).unwrap_err();
    assert!(err.contains("BadVoucher"), "89 bytes: {err}");

    // Fewer than 16 bytes: no room for a header. The ed25519 program may
    // refuse it first; either way nothing is paid.
    let v = Instruction { program_id: addr(ED25519), accounts: vec![], data: vec![1, 0, 48, 0] };
    assert!(pay_with(&mut c, v, 1_000).is_err());
    assert_eq!(payee_usd(&c), 0);

    pay_with(&mut c, ed25519_raw(1, [48, u16::MAX, 16, u16::MAX, 112, 90, u16::MAX], &tail), 1_000).unwrap();
    assert_eq!(payee_usd(&c), 1_000);
}

#[test]
fn the_payee_may_close_after_the_notice_too() {
    let mut c = setup();
    pay(&mut c, 100_000).unwrap();
    let payer = c.payer.insecure_clone();
    try_ix(&mut c.w.svm, ask_to_close_channel_ix(payer.pubkey(), c.ch), &payer).unwrap();
    let asked = channel(&c.w.svm, c.ch).asked;
    c.w.svm.warp_to_slot(asked + sasona::NOTICE_SLOTS + 10);
    let payee = c.payee.insecure_clone();
    close_by(&mut c, &payee).unwrap();
    assert_eq!(usd_balance(&c.w.svm, payer.pubkey()), 10 * DOLLAR - 100_000 - 15_000);
}

#[test]
fn entry_fees_and_markup_share_the_fee_account_without_spending_each_other() {
    let mut c = setup();
    // Entry fees waiting, from a deposit, and markup waiting, from a channel.
    let (k, from) = newcomer(&mut c.w.svm, 100);
    try_deposit(&mut c.w.svm, &k, from, 100 * DOLLAR).unwrap();
    pay(&mut c, 600_000).unwrap();
    sweep(&mut c).unwrap();
    let entry = pool(&c.w.svm).fees_held;
    assert!(entry > 0);
    assert_eq!(markup_held(&c.w.svm), 90_000);
    let fees_before = fees(&c);
    let caller = c.payee.insecure_clone();
    try_ix(&mut c.w.svm, settle_ix(caller.pubkey()), &caller).unwrap();
    assert_eq!(fees(&c), fees_before - entry);
    assert_eq!(markup_held(&c.w.svm), 90_000, "the entry fees took nothing of the markup");
    let network_before = token_balance(&c.w.svm, pda(&[NETWORK_SEED]));
    try_ix(&mut c.w.svm, settle_markup_ix(caller.pubkey()), &caller).unwrap();
    assert_eq!(fees(&c), fees_before - entry - 90_000);
    assert!(token_balance(&c.w.svm, pda(&[NETWORK_SEED])) > network_before, "the participants' share went to the network");
    assert_books_balance(&c.w.svm);
}
