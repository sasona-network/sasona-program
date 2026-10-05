//! Send Sasona instructions to devnet.
//!
//!     sasona open <dollars> --keypair <path>
//!     sasona deposit <dollars> --keypair <path>
//!     sasona fee <dollars of markup> --keypair <path>
//!     sasona settle --keypair <path>
//!     sasona depth <dollars> --keypair <path>
//!     sasona join <owner> --keypair <path>
//!     sasona ask-back <shares> --keypair <path>
//!     sasona release --keypair <path>
//!     sasona open-round <candidates file> <count> --keypair <path>
//!     sasona reveal-round <candidates file> --keypair <path>
//!     sasona withheld <candidates file> --keypair <path>
//!     sasona commit-reading <round> <service url> <question hash hex> --keypair <path>
//!     sasona reveal-reading <round> <service url> <nonce hex> <reply hash hex> <verdict> --keypair <path>
//!     sasona commit-second <re-read round> <service url> <question hash hex> <first reading> --keypair <path>
//!     sasona settle-pair <first reading> <second reading> --keypair <path>
//!     sasona member-join --keypair <path>
//!     sasona member-ask-leave <number> --keypair <path>
//!     sasona member-leave <number> --keypair <path>
//!     sasona challenge <reading> --keypair <path>
//!     sasona evidence <reading> <reply file> --keypair <path>
//!     sasona answer <reading> <nonce hex> --keypair <path>
//!     sasona uphold <reading> --keypair <path>
//!     sasona quote <reading> <basis points, 0 to withdraw> --keypair <path>
//!     sasona buy <reading> <purchase id> <dollars> [highest rate] --keypair <path>
//!     sasona charge-back <purchase> --keypair <path>
//!     sasona record-draw <purchase> --keypair <path>
//!     sasona commit-replay <purchase> <service url> <question hash hex> --keypair <path>
//!     sasona reveal-replay <purchase> <nonce hex> <reply hash hex> <verdict> --keypair <path>
//!     sasona pass-draw <purchase> --keypair <path>
//!     sasona settle-chargeback <purchase> --keypair <path>
//!     sasona repay-cover <purchase> --keypair <path>
//!     sasona open-channel <payee> <id> <dollars> [--signer <path>] --keypair <path>
//!     sasona add-to-channel <channel> <dollars> --keypair <path>
//!     sasona voucher <channel> <units> --keypair <signer path>
//!     sasona take-payment <channel> <units> <signature hex> --keypair <path>
//!     sasona sweep-channel <channel> --keypair <path>
//!     sasona ask-close-channel <channel> --keypair <path>
//!     sasona close-channel <channel> --keypair <path>
//!     sasona settle-markup --keypair <path>
//!
//! voucher signs offline and prints the signature: the 90 bytes of
//! sasona-protocol SPEC.md 8.3, for the channel and the amount in dollar
//! units (a millionth of a dollar), everything taken so far. take-payment
//! puts that signature before the payment, for the ed25519 program to check.
//!
//! reveal-reading and reveal-replay take [--pay-to <address>]: where the
//! service asked to be paid (sasona-protocol SPEC.md 7.1).
//!
//! commit-reading and commit-second work out the draw for the service
//! (sasona-protocol SPEC.md 4.3) and send the keypair's membership with the
//! memberships passed over before it. They stop if the keypair was not drawn.
//!
//! A round is found by its list's fingerprint. open-round keeps the seed in
//! `<keypair>.round-<fingerprint>.seed` until it is revealed; anyone who
//! reads that file early can see the draw coming.
//!
//! The keypair signs and pays. It must hold the devnet dollar in its
//! associated token account. Prints the transaction signature.

use std::rc::Rc;

use anchor_client::anchor_lang::prelude::Pubkey;
use anchor_client::{Client, Cluster, CommitmentConfig};
use anchor_spl::associated_token::get_associated_token_address;
use anchor_client::anchor_lang::prelude::AccountMeta;
use sasona::{
    COIN_SEED, COVER_SEED, COVER_VAULT_SEED, EXIT_SEED, FEES_SEED, GUARANTEE_SEED, NETWORK_SEED, POOL_COIN_SEED,
    NONCE_SEED, PAIR_SEED, POOL_SEED, POOL_USD_SEED, READING_SEED, ROUND_SEED, USD_MINT, VAULT_SEED,
    CHALLENGE_SEED, EVIDENCE_SEED, QUOTE_SEED, MARKUP_SEED, CHANNEL_SEED, CHANNEL_USD_SEED, PAYER_SEED, BOOK_SEED, CHARGEBACK_SEED, ESCROW_SEED, PURCHASE_SEED, SERVICE_SEED, HELD_SEED, MEMBERS_SEED, MEMBER_SEED, SEAT_SEED, STAKES_SEED,
};
use solana_keypair::read_keypair_file;
use sha2::Digest;
use solana_signer::Signer;

const USAGE: &str = "usage: sasona <open|deposit|fee|depth> <dollars> --keypair <path>
       sasona settle|release --keypair <path>
       sasona join <owner> --keypair <path>
       sasona ask-back <shares> --keypair <path>
       sasona open-round <candidates file> <count> | reveal-round <candidates file> | withheld <candidates file> --keypair <path>
       (all take [--url <rpc>])";

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn unhex<const N: usize>(s: &str) -> [u8; N] {
    assert_eq!(s.len(), 2 * N, "expected {} hex characters", 2 * N);
    let mut out = [0u8; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("hex");
    }
    out
}

fn candidates_in(path: &str) -> Vec<String> {
    let list = std::fs::read_to_string(path).expect("candidates file");
    list.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
}

/// sasona-protocol SPEC.md 1.2.
fn fingerprint_of(path: &str) -> [u8; 32] {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    for c in candidates_in(path) {
        h.update(c.as_bytes());
        h.update([0u8]);
    }
    h.finalize().into()
}

fn seed_file(keypair_path: &str, fingerprint: &[u8; 32]) -> String {
    let hex: String = fingerprint.iter().map(|b| format!("{b:02x}")).collect();
    format!("{keypair_path}.round-{hex}.seed")
}

const ED25519: Pubkey = anchor_client::anchor_lang::pubkey!("Ed25519SigVerify111111111111111111111111111");
const INSTRUCTIONS: Pubkey = anchor_client::anchor_lang::pubkey!("Sysvar1nstructions1111111111111111111111111");

fn pda(seeds: &[&[u8]]) -> Pubkey {
    Pubkey::find_program_address(seeds, &sasona::ID).0
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).cloned().unwrap_or_default();
    if !["open", "deposit", "fee", "settle", "depth", "join", "ask-back", "release", "open-round", "reveal-round", "withheld", "commit-reading", "reveal-reading", "commit-second", "settle-pair", "member-join", "member-ask-leave", "member-leave", "challenge", "evidence", "answer", "uphold", "quote", "buy", "charge-back", "record-draw", "commit-replay", "reveal-replay", "pass-draw", "settle-chargeback", "repay-cover", "open-channel", "add-to-channel", "voucher", "take-payment", "sweep-channel", "ask-close-channel", "close-channel", "settle-markup"].contains(&command.as_str()) {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }
    let dollars: f64 = if ["settle", "join", "release", "open-round", "reveal-round", "withheld", "commit-reading", "reveal-reading", "commit-second", "settle-pair", "member-join", "member-ask-leave", "member-leave", "challenge", "evidence", "answer", "uphold", "quote", "buy", "charge-back", "record-draw", "commit-replay", "reveal-replay", "pass-draw", "settle-chargeback", "repay-cover", "open-channel", "add-to-channel", "voucher", "take-payment", "sweep-channel", "ask-close-channel", "close-channel", "settle-markup"].contains(&command.as_str()) { 0.0 } else { args.get(2).and_then(|s| s.parse().ok()).expect(USAGE) };
    let amount = (dollars * 1_000_000.0).round() as u64;
    let keypair_path = arg(&args, "--keypair").expect(USAGE);
    let keypair = read_keypair_file(&keypair_path).expect("cannot read keypair");
    let cluster = match arg(&args, "--url") {
        Some(url) => Cluster::Custom(url.clone(), url.replace("https", "wss")),
        None => Cluster::Devnet,
    };

    // SPEC.md 8.3: a voucher is a signature, made offline.
    if command == "voucher" {
        let channel: Pubkey = args[2].parse().expect("channel address");
        let units: u64 = args[3].parse().expect("amount in dollar units");
        let signature = keypair.sign_message(&sasona::voucher_message(&channel, units));
        println!("{}", signature.as_ref().iter().map(|b| format!("{b:02x}")).collect::<String>());
        return;
    }
    let voucher_signer = arg(&args, "--signer").map(|p| read_keypair_file(&p).expect("cannot read signer keypair"));

    let me = keypair.pubkey();
    let client = Client::new_with_options(cluster, Rc::new(keypair), CommitmentConfig::finalized());
    let program = client.program(sasona::ID).expect("program client");
    let coin = pda(&[COIN_SEED]);

    let member_pda = |n: u32| pda(&[MEMBER_SEED, &n.to_le_bytes()]);
    let seat_pda = |k: u32| pda(&[SEAT_SEED, &k.to_le_bytes()]);
    let members_now = || -> sasona::Members { program.account(pda(&[MEMBERS_SEED])).expect("members") };
    // SPEC.md 4.3: this keypair's membership if its seat was drawn for the
    // service, and the seats passed over that have to be shown.
    let drawn = |round: &Pubkey, endpoint_hash: &[u8; 32], first_reader: Option<Pubkey>| -> (Pubkey, Pubkey, Vec<AccountMeta>) {
        let r: sasona::Round = program.account(*round).expect("round");
        assert!(r.members > 0, "nobody was a member when this round was opened");
        let seated = members_now().seated;
        let mut skipped = vec![];
        for a in 0..sasona::MAX_READER_ATTEMPTS {
            let k = sasona::reader_number(&r.final_seed, endpoint_hash, a, r.members);
            if k > seated {
                continue;
            }
            let s: sasona::Seat = program.account(seat_pda(k)).expect("seat");
            if s.since >= r.commit_slot || Some(s.owner) == first_reader {
                skipped.push(AccountMeta::new_readonly(seat_pda(k), false));
                continue;
            }
            assert_eq!(s.owner, me, "seat {k} was drawn for this service, and it is not yours");
            eprintln!("drawn: seat {k}, membership {}", s.member);
            return (member_pda(s.member), seat_pda(k), skipped);
        }
        panic!("no seat qualified in {} attempts: nobody reads this service in this round", sasona::MAX_READER_ATTEMPTS);
    };
    // SPEC.md 4.2: the accounts that take a membership off the roster.
    let unseat = |number: u32| -> Vec<AccountMeta> {
        let k: u32 = { let m: sasona::Member = program.account(member_pda(number)).expect("membership"); m.seat };
        if k == 0 {
            return vec![];
        }
        let last = members_now().seated;
        let mut out = vec![AccountMeta::new(seat_pda(k), false)];
        if k != last {
            let l: sasona::Seat = program.account(seat_pda(last)).expect("last seat");
            out.push(AccountMeta::new(seat_pda(last), false));
            out.push(AccountMeta::new(member_pda(l.member), false));
        }
        out
    };
    let reading_of = |reading: &Pubkey| -> sasona::Reading { program.account(*reading).expect("reading") };
    let pay_to = || -> Pubkey { arg(&args, "--pay-to").map(|s| s.parse().expect("pay-to address")).unwrap_or_default() };
    let chargeback_of = |purchase: &Pubkey| -> (Pubkey, sasona::Chargeback) {
        let at = pda(&[CHARGEBACK_SEED, purchase.as_ref()]);
        (at, program.account(at).expect("chargeback"))
    };
    let replay_pda = |chargeback: &Pubkey, draw: u8| pda(&[READING_SEED, chargeback.as_ref(), &[draw]]);
    // SPEC.md 7.4: the seat drawn for a chargeback's current draw, and the
    // seats passed over before it, once its entropy is recorded.
    let replay_drawn = |c: &sasona::Chargeback| -> (Option<u32>, Vec<u32>) {
        let mut skipped = vec![];
        if c.draw_members == 0 {
            return (None, skipped);
        }
        let seated = members_now().seated;
        let declined = &c.declined[..c.counted_draws as usize];
        for a in 0..sasona::MAX_READER_ATTEMPTS {
            let k = sasona::reader_number(&c.seed, &c.service, a, c.draw_members);
            if k > seated {
                continue;
            }
            let s: sasona::Seat = program.account(seat_pda(k)).expect("seat");
            if s.since >= c.draw_slot || s.owner == c.buyer || s.owner == c.quoter || declined.contains(&s.member) {
                skipped.push(k);
                continue;
            }
            return (Some(k), skipped);
        }
        (None, skipped)
    };

    if command == "evidence" {
        // Several transactions: room for the reply, then the reply in pieces.
        let reading: Pubkey = args[2].parse().expect("reading address");
        let reply = std::fs::read(&args[3]).expect("reply file");
        let evidence = pda(&[EVIDENCE_SEED, reading.as_ref()]);
        let mut last = program
            .request()
            .accounts(sasona::accounts::OpenEvidence { reader: me, reading, evidence, system_program: anchor_client::anchor_lang::system_program::ID })
            .args(sasona::instruction::OpenEvidence { len: reply.len() as u32 })
            .send()
            .expect("open-evidence failed");
        for (i, chunk) in reply.chunks(800).enumerate() {
            last = program
                .request()
                .accounts(sasona::accounts::WriteEvidence { reader: me, reading, evidence })
                .args(sasona::instruction::WriteEvidence { offset: (i * 800) as u32, bytes: chunk.to_vec() })
                .send()
                .expect("write-evidence failed");
        }
        println!("{last}");
        return;
    }

    let request = program.request();
    let request = if command == "member-join" {
        let members: Option<sasona::Members> = program.account(pda(&[MEMBERS_SEED])).ok();
        let number = members.as_ref().map(|m| m.count).unwrap_or(0) + 1;
        let seat = members.as_ref().map(|m| m.seated).unwrap_or(0) + 1;
        eprintln!("membership {number}, seat {seat}");
        request
            .accounts(sasona::accounts::JoinMembers {
                owner: me,
                pool: pda(&[POOL_SEED]),
                coin_mint: coin,
                owner_coin: get_associated_token_address(&me, &coin),
                members: pda(&[MEMBERS_SEED]),
                member: member_pda(number),
                seat: seat_pda(seat),
                stakes: pda(&[STAKES_SEED]),
                held: pda(&[HELD_SEED]),
                token_program: anchor_spl::token::ID,
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .args(sasona::instruction::JoinMembers {})
    } else if command == "member-ask-leave" {
        let number: u32 = args[2].parse().expect("membership number");
        request
            .accounts(sasona::accounts::AskToLeave { owner: me, members: pda(&[MEMBERS_SEED]), member: member_pda(number) })
            .accounts(unseat(number))
            .args(sasona::instruction::AskToLeave {})
    } else if command == "member-leave" {
        let number: u32 = args[2].parse().expect("membership number");
        request
            .accounts(sasona::accounts::Leave {
                owner: me,
                pool: pda(&[POOL_SEED]),
                coin_mint: coin,
                member: member_pda(number),
                stakes: pda(&[STAKES_SEED]),
                owner_coin: get_associated_token_address(&me, &coin),
                book: pda(&[BOOK_SEED, &number.to_le_bytes()]),
                token_program: anchor_spl::token::ID,
            })
            .args(sasona::instruction::Leave {})
    } else if command == "challenge" {
        let reading: Pubkey = args[2].parse().expect("reading address");
        let r = reading_of(&reading);
        request
            .accounts(sasona::accounts::ChallengeReading {
                challenger: me,
                reading,
                member: member_pda(r.member),
                challenge: pda(&[CHALLENGE_SEED, reading.as_ref()]),
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .args(sasona::instruction::Challenge {})
    } else if command == "answer" {
        let reading: Pubkey = args[2].parse().expect("reading address");
        let nonce = unhex::<16>(&args[3]);
        let r = reading_of(&reading);
        request
            .accounts(sasona::accounts::AnswerChallenge {
                challenge: pda(&[CHALLENGE_SEED, reading.as_ref()]),
                reading,
                evidence: pda(&[EVIDENCE_SEED, reading.as_ref()]),
                used_nonce: pda(&[NONCE_SEED, &nonce]),
                member: member_pda(r.member),
                reader: r.reader,
            })
            .args(sasona::instruction::AnswerChallenge { nonce })
    } else if command == "uphold" {
        let reading: Pubkey = args[2].parse().expect("reading address");
        let r = reading_of(&reading);
        let c: sasona::Challenge = program.account(pda(&[CHALLENGE_SEED, reading.as_ref()])).expect("challenge");
        request
            .accounts(sasona::accounts::UpholdChallenge {
                challenge: pda(&[CHALLENGE_SEED, reading.as_ref()]),
                reading,
                members: pda(&[MEMBERS_SEED]),
                member: member_pda(r.member),
                pool: pda(&[POOL_SEED]),
                coin_mint: coin,
                stakes: pda(&[STAKES_SEED]),
                cover: pda(&[COVER_SEED]),
                cover_vault: pda(&[COVER_VAULT_SEED]),
                book: pda(&[BOOK_SEED, &r.member.to_le_bytes()]),
                challenger: c.challenger,
                challenger_coin: get_associated_token_address(&c.challenger, &coin),
                token_program: anchor_spl::token::ID,
            })
            .accounts(unseat(r.member))
            .args(sasona::instruction::UpholdChallenge {})
    } else if command == "buy" {
        let reading: Pubkey = args[2].parse().expect("reading address");
        let id: u64 = args[3].parse().expect("purchase id");
        let price = (args[4].parse::<f64>().expect("dollars") * 1_000_000.0).round() as u64;
        let r = reading_of(&reading);
        let quoter: sasona::Member = program.account(member_pda(r.member)).expect("membership");
        // The rate the buyer accepts: the one standing now, unless named.
        let max_rate: u16 = match args.get(5) {
            Some(a) if !a.starts_with("--") => a.parse().expect("basis points"),
            _ => program.account::<sasona::Quote>(pda(&[QUOTE_SEED, reading.as_ref()])).expect("quote").rate,
        };
        eprintln!("highest rate accepted {max_rate} bps");
        eprintln!("purchase {}", pda(&[PURCHASE_SEED, me.as_ref(), &id.to_le_bytes()]));
        request
            .accounts(sasona::accounts::Buy {
                buyer: me,
                buyer_usd: get_associated_token_address(&me, &USD_MINT),
                usd_mint: USD_MINT,
                pool: pda(&[POOL_SEED]),
                reading,
                quote: pda(&[QUOTE_SEED, reading.as_ref()]),
                member: member_pda(r.member),
                book: pda(&[BOOK_SEED, &r.member.to_le_bytes()]),
                purchase: pda(&[PURCHASE_SEED, me.as_ref(), &id.to_le_bytes()]),
                merchant_usd: get_associated_token_address(&r.pay_to, &USD_MINT),
                quoter_usd: get_associated_token_address(&quoter.owner, &USD_MINT),
                fees: pda(&[FEES_SEED]),
                markup: pda(&[MARKUP_SEED]),
                token_program: anchor_spl::token::ID,
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .args(sasona::instruction::Buy { id, price, max_rate })
    } else if command == "charge-back" {
        let purchase: Pubkey = args[2].parse().expect("purchase address");
        let pu: sasona::Purchase = program.account(purchase).expect("purchase");
        request
            .accounts(sasona::accounts::ChargeBack {
                buyer: me,
                buyer_usd: get_associated_token_address(&me, &USD_MINT),
                usd_mint: USD_MINT,
                pool: pda(&[POOL_SEED]),
                purchase,
                member: member_pda(pu.member),
                members: pda(&[MEMBERS_SEED]),
                service_terms: pda(&[SERVICE_SEED, &pu.service]),
                chargeback: pda(&[CHARGEBACK_SEED, purchase.as_ref()]),
                escrow: pda(&[ESCROW_SEED]),
                token_program: anchor_spl::token::ID,
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .args(sasona::instruction::ChargeBack {})
    } else if command == "record-draw" {
        let purchase: Pubkey = args[2].parse().expect("purchase address");
        request
            .accounts(sasona::accounts::RecordDraw {
                chargeback: pda(&[CHARGEBACK_SEED, purchase.as_ref()]),
                slot_hashes: sasona::SLOT_HASHES_ID,
            })
            .args(sasona::instruction::RecordDraw {})
    } else if command == "commit-replay" {
        let purchase: Pubkey = args[2].parse().expect("purchase address");
        let (at, c) = chargeback_of(&purchase);
        assert!(c.seed != [0u8; 32], "record the draw first: sasona record-draw {purchase}");
        let (drawn, skipped) = replay_drawn(&c);
        let k = drawn.expect("no seat qualifies for this draw; pass it");
        let s: sasona::Seat = program.account(seat_pda(k)).expect("seat");
        assert_eq!(s.owner, me, "seat {k} was drawn to replay, and it is not yours");
        eprintln!("drawn: seat {k}, membership {}", s.member);
        request
            .accounts(sasona::accounts::CommitReplay {
                reader: me,
                chargeback: at,
                members: pda(&[MEMBERS_SEED]),
                member: member_pda(s.member),
                seat: seat_pda(k),
                replay: replay_pda(&at, c.draw),
                slot_hashes: sasona::SLOT_HASHES_ID,
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .accounts(skipped.into_iter().map(|k| AccountMeta::new_readonly(seat_pda(k), false)).collect::<Vec<_>>())
            .args(sasona::instruction::CommitReplay { endpoint: args[3].clone(), question_hash: unhex::<32>(&args[4]) })
    } else if command == "reveal-replay" {
        let purchase: Pubkey = args[2].parse().expect("purchase address");
        let (at, c) = chargeback_of(&purchase);
        let nonce = unhex::<16>(&args[3]);
        request
            .accounts(sasona::accounts::RevealReading {
                reader: me,
                reading: replay_pda(&at, c.draw),
                used_nonce: pda(&[NONCE_SEED, &nonce]),
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .args(sasona::instruction::RevealReading {
                nonce,
                reply_hash: unhex::<32>(&args[4]),
                verdict: args[5].parse().expect("verdict"),
                pay_to: pay_to(),
            })
    } else if command == "pass-draw" {
        let purchase: Pubkey = args[2].parse().expect("purchase address");
        let (at, c) = chargeback_of(&purchase);
        let (drawn, skipped) = if c.seed == [0u8; 32] { (None, vec![]) } else { replay_drawn(&c) };
        let mut shown: Vec<AccountMeta> = drawn.iter().map(|&k| AccountMeta::new_readonly(seat_pda(k), false)).collect();
        shown.extend(skipped.into_iter().map(|k| AccountMeta::new_readonly(seat_pda(k), false)));
        request
            .accounts(sasona::accounts::PassDraw {
                chargeback: at,
                members: pda(&[MEMBERS_SEED]),
                replay: replay_pda(&at, c.draw),
                slot_hashes: sasona::SLOT_HASHES_ID,
            })
            .accounts(shown)
            .args(sasona::instruction::PassDraw { drawn_seat: drawn.unwrap_or(0) })
    } else if command == "settle-chargeback" {
        let purchase: Pubkey = args[2].parse().expect("purchase address");
        let (at, c) = chargeback_of(&purchase);
        let pu: sasona::Purchase = program.account(purchase).expect("purchase");
        let replay = replay_pda(&at, c.draw);
        let replayer = program
            .account::<sasona::Reading>(replay)
            .ok()
            .filter(|r| r.state == sasona::READING_REVEALED)
            .map(|r| get_associated_token_address(&r.reader, &USD_MINT));
        request
            .accounts(sasona::accounts::SettleChargeback {
                chargeback: at,
                purchase,
                book: pda(&[BOOK_SEED, &pu.member.to_le_bytes()]),
                replay,
                pool: pda(&[POOL_SEED]),
                coin_mint: coin,
                pool_usd: pda(&[POOL_USD_SEED]),
                pool_coin: pda(&[POOL_COIN_SEED]),
                fees: pda(&[FEES_SEED]),
                cover: pda(&[COVER_SEED]),
                cover_vault: pda(&[COVER_VAULT_SEED]),
                escrow: pda(&[ESCROW_SEED]),
                buyer_usd: get_associated_token_address(&c.buyer, &USD_MINT),
                replayer_usd: replayer,
                token_program: anchor_spl::token::ID,
            })
            .args(sasona::instruction::SettleChargeback {})
    } else if command == "repay-cover" {
        let purchase: Pubkey = args[2].parse().expect("purchase address");
        let (at, c) = chargeback_of(&purchase);
        let m: sasona::Member = program.account(member_pda(c.member)).expect("membership");
        // Left with less than a whole stake, the membership gives up its seat.
        let drained = m.seat > 0 && m.stake.saturating_sub(c.owed_coins) < sasona::MEMBER_STAKE;
        request
            .accounts(sasona::accounts::RepayCover {
                chargeback: at,
                book: pda(&[BOOK_SEED, &c.member.to_le_bytes()]),
                member: member_pda(c.member),
                members: pda(&[MEMBERS_SEED]),
                pool: pda(&[POOL_SEED]),
                coin_mint: coin,
                stakes: pda(&[STAKES_SEED]),
                cover: pda(&[COVER_SEED]),
                cover_vault: pda(&[COVER_VAULT_SEED]),
                token_program: anchor_spl::token::ID,
            })
            .accounts(if drained { unseat(c.member) } else { vec![] })
            .args(sasona::instruction::RepayCover {})
    } else if command == "quote" {
        let reading: Pubkey = args[2].parse().expect("reading address");
        let rate: u16 = args[3].parse().expect("basis points");
        let r = reading_of(&reading);
        request
            .accounts(sasona::accounts::SetQuote {
                reader: me,
                reading,
                member: member_pda(r.member),
                quote: pda(&[QUOTE_SEED, reading.as_ref()]),
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .args(sasona::instruction::SetQuote { rate })
    } else if command == "open" {
        request
            .accounts(sasona::accounts::Open {
                depositor: me,
                pool: pda(&[POOL_SEED]),
                coin_mint: coin,
                usd_mint: USD_MINT,
                pool_usd: pda(&[POOL_USD_SEED]),
                pool_coin: pda(&[POOL_COIN_SEED]),
                fees: pda(&[FEES_SEED]),
                depositor_usd: get_associated_token_address(&me, &USD_MINT),
                depositor_coin: get_associated_token_address(&me, &coin),
                cover: pda(&[COVER_SEED]),
                cover_vault: pda(&[COVER_VAULT_SEED]),
                guarantee: pda(&[GUARANTEE_SEED, me.as_ref()]),
                token_program: anchor_spl::token::ID,
                associated_token_program: anchor_spl::associated_token::ID,
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .args(sasona::instruction::Open { amount })
    } else if command == "fee" {
        request
            .accounts(sasona::accounts::PayFee {
                payer: me,
                pool: pda(&[POOL_SEED]),
                coin_mint: coin,
                usd_mint: USD_MINT,
                pool_usd: pda(&[POOL_USD_SEED]),
                pool_coin: pda(&[POOL_COIN_SEED]),
                payer_usd: get_associated_token_address(&me, &USD_MINT),
                fees: pda(&[FEES_SEED]),
                network: pda(&[NETWORK_SEED]),
                token_program: anchor_spl::token::ID,
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .args(sasona::instruction::PayFee { markup: amount })
    } else if command == "depth" {
        request
            .accounts(sasona::accounts::AddDepth {
                giver: me,
                pool: pda(&[POOL_SEED]),
                coin_mint: coin,
                usd_mint: USD_MINT,
                pool_usd: pda(&[POOL_USD_SEED]),
                fees: pda(&[FEES_SEED]),
                giver_usd: get_associated_token_address(&me, &USD_MINT),
                token_program: anchor_spl::token::ID,
            })
            .args(sasona::instruction::AddDepth { amount })
    } else if command == "join" {
        let owner: Pubkey = args[2].parse().expect("owner address");
        request
            .accounts(sasona::accounts::JoinCover {
                caller: me,
                pool: pda(&[POOL_SEED]),
                coin_mint: coin,
                cover: pda(&[COVER_SEED]),
                cover_vault: pda(&[COVER_VAULT_SEED]),
                owner,
                guarantee: pda(&[GUARANTEE_SEED, owner.as_ref()]),
                legacy_vault: pda(&[VAULT_SEED, owner.as_ref()]),
                token_program: anchor_spl::token::ID,
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .args(sasona::instruction::JoinCover {})
    } else if command == "ask-back" {
        let shares: u64 = args[2].parse().expect("shares");
        request
            .accounts(sasona::accounts::RequestRelease {
                owner: me,
                guarantee: pda(&[GUARANTEE_SEED, me.as_ref()]),
                exit: pda(&[EXIT_SEED, me.as_ref()]),
                legacy_vault: pda(&[VAULT_SEED, me.as_ref()]),
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .args(sasona::instruction::RequestRelease { shares })
    } else if command == "release" {
        request
            .accounts(sasona::accounts::Release {
                owner: me,
                pool: pda(&[POOL_SEED]),
                coin_mint: coin,
                cover: pda(&[COVER_SEED]),
                cover_vault: pda(&[COVER_VAULT_SEED]),
                exit: pda(&[EXIT_SEED, me.as_ref()]),
                owner_coin: get_associated_token_address(&me, &coin),
                token_program: anchor_spl::token::ID,
                associated_token_program: anchor_spl::associated_token::ID,
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .args(sasona::instruction::Release {})
    } else if command == "open-round" {
        let fingerprint = fingerprint_of(&args[2]);
        let size = candidates_in(&args[2]).len() as u32;
        let count: u16 = args[3].parse().expect("count");
        let mut seed = [0u8; 32];
        std::io::Read::read_exact(&mut std::fs::File::open("/dev/urandom").expect("urandom"), &mut seed).unwrap();
        let seed_file = seed_file(&keypair_path, &fingerprint);
        assert!(!std::path::Path::new(&seed_file).exists(), "a seed for this list already exists");
        std::fs::write(&seed_file, seed).expect("write seed");
        let seed_hash: [u8; 32] = sha2::Sha256::digest(seed).into();
        eprintln!("round {} over {size} candidates, {count} picks", pda(&[ROUND_SEED, &fingerprint]));
        request
            .accounts(sasona::accounts::OpenRound {
                members: pda(&[MEMBERS_SEED]),
                opener: me,
                round: pda(&[ROUND_SEED, &fingerprint]),
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .args(sasona::instruction::OpenRound { pool_fingerprint: fingerprint, pool_size: size, count, seed_hash })
    } else if command == "reveal-round" {
        let fingerprint = fingerprint_of(&args[2]);
        let seed: [u8; 32] = std::fs::read(seed_file(&keypair_path, &fingerprint))
            .expect("seed file")
            .try_into()
            .expect("32 bytes");
        request
            .accounts(sasona::accounts::RevealRound {
                round: pda(&[ROUND_SEED, &fingerprint]),
                opener: me,
                slot_hashes: sasona::SLOT_HASHES_ID,
            })
            .args(sasona::instruction::RevealRound { seed })
    } else if command == "withheld" {
        let fingerprint = fingerprint_of(&args[2]);
        request
            .accounts(sasona::accounts::MarkWithheld {
                round: pda(&[ROUND_SEED, &fingerprint]),
                slot_hashes: sasona::SLOT_HASHES_ID,
            })
            .args(sasona::instruction::MarkWithheld {})
    } else if command == "commit-reading" {
        let round: Pubkey = args[2].parse().expect("round address");
        let endpoint = args[3].clone();
        let endpoint_hash: [u8; 32] = sha2::Sha256::digest(endpoint.as_bytes()).into();
        let (member, seat, skipped) = drawn(&round, &endpoint_hash, None);
        request
            .accounts(sasona::accounts::CommitReading {
                reader: me,
                round,
                members: pda(&[MEMBERS_SEED]),
                member,
                seat,
                reading: pda(&[READING_SEED, round.as_ref(), &endpoint_hash]),
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .accounts(skipped)
            .args(sasona::instruction::CommitReading { endpoint_hash, endpoint, question_hash: unhex::<32>(&args[4]) })
    } else if command == "reveal-reading" {
        let round: Pubkey = args[2].parse().expect("round address");
        let endpoint_hash: [u8; 32] = sha2::Sha256::digest(args[3].as_bytes()).into();
        let nonce = unhex::<16>(&args[4]);
        request
            .accounts(sasona::accounts::RevealReading {
                reader: me,
                reading: pda(&[READING_SEED, round.as_ref(), &endpoint_hash]),
                used_nonce: pda(&[NONCE_SEED, &nonce]),
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .args(sasona::instruction::RevealReading {
                nonce,
                reply_hash: unhex::<32>(&args[5]),
                verdict: args[6].parse().expect("verdict"),
                pay_to: pay_to(),
            })
    } else if command == "commit-second" {
        let round: Pubkey = args[2].parse().expect("re-read round address");
        let endpoint = args[3].clone();
        let endpoint_hash: [u8; 32] = sha2::Sha256::digest(endpoint.as_bytes()).into();
        let reading = pda(&[READING_SEED, round.as_ref(), &endpoint_hash]);
        let first: Pubkey = args[5].parse().expect("first reading address");
        let (member, seat, skipped) = drawn(&round, &endpoint_hash, Some(reading_of(&first).reader));
        request
            .accounts(sasona::accounts::CommitSecondReading {
                reader: me,
                round,
                members: pda(&[MEMBERS_SEED]),
                member,
                seat,
                reading,
                first,
                pair: pda(&[PAIR_SEED, reading.as_ref()]),
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .accounts(skipped)
            .args(sasona::instruction::CommitSecondReading { endpoint_hash, endpoint, question_hash: unhex::<32>(&args[4]) })
    } else if command == "settle-pair" {
        let second: Pubkey = args[3].parse().expect("second reading address");
        request
            .accounts(sasona::accounts::SettlePair {
                pair: pda(&[PAIR_SEED, second.as_ref()]),
                first: args[2].parse().expect("first reading address"),
                second,
            })
            .args(sasona::instruction::SettlePair {})
    } else if command == "open-channel" {
        let payee: Pubkey = args[2].parse().expect("payee address");
        let id: u64 = args[3].parse().expect("identifier");
        let put_in = (args[4].parse::<f64>().expect("dollars") * 1_000_000.0).round() as u64;
        let signer = voucher_signer.as_ref().map(|k| k.pubkey()).unwrap_or(me);
        let channel = pda(&[CHANNEL_SEED, me.as_ref(), payee.as_ref(), &id.to_le_bytes()]);
        eprintln!("channel {channel}");
        let r = request
            .accounts(sasona::accounts::OpenChannel {
                payer: me,
                voucher_signer: signer,
                payer_usd: get_associated_token_address(&me, &USD_MINT),
                usd_mint: USD_MINT,
                record: pda(&[PAYER_SEED, me.as_ref()]),
                channel,
                channel_usd: pda(&[CHANNEL_USD_SEED, channel.as_ref()]),
                token_program: anchor_spl::token::ID,
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .args(sasona::instruction::OpenChannel { payee, id, amount: put_in });
        match &voucher_signer {
            Some(k) => r.signer(k.insecure_clone()),
            None => r,
        }
    } else if command == "add-to-channel" {
        let channel: Pubkey = args[2].parse().expect("channel address");
        let more = (args[3].parse::<f64>().expect("dollars") * 1_000_000.0).round() as u64;
        request
            .accounts(sasona::accounts::AddToChannel {
                payer: me,
                payer_usd: get_associated_token_address(&me, &USD_MINT),
                channel,
                channel_usd: pda(&[CHANNEL_USD_SEED, channel.as_ref()]),
                token_program: anchor_spl::token::ID,
            })
            .args(sasona::instruction::AddToChannel { amount: more })
    } else if command == "take-payment" {
        let channel: Pubkey = args[2].parse().expect("channel address");
        let units: u64 = args[3].parse().expect("amount in dollar units");
        let signature = unhex::<64>(&args[4]);
        let c: sasona::Channel = program.account(channel).expect("channel");
        // SPEC.md 8.6: one signature, with the key, the signature and the
        // message in this same instruction: key at 16, signature at 48,
        // message at 112.
        let mut data = vec![1u8, 0];
        for f in [48u16, u16::MAX, 16, u16::MAX, 112, sasona::VOUCHER_LEN as u16, u16::MAX] {
            data.extend(f.to_le_bytes());
        }
        data.extend(c.signer.as_ref());
        data.extend(signature);
        data.extend(sasona::voucher_message(&channel, units));
        let check = anchor_client::Instruction { program_id: ED25519, accounts: vec![], data };
        request
            .instruction(check)
            .accounts(sasona::accounts::TakePayment {
                channel,
                channel_usd: pda(&[CHANNEL_USD_SEED, channel.as_ref()]),
                payee_usd: get_associated_token_address(&c.payee, &USD_MINT),
                instructions: INSTRUCTIONS,
                token_program: anchor_spl::token::ID,
            })
            .args(sasona::instruction::TakePayment { amount: units })
    } else if command == "sweep-channel" {
        let channel: Pubkey = args[2].parse().expect("channel address");
        request
            .accounts(sasona::accounts::SweepChannel {
                caller: me,
                channel,
                channel_usd: pda(&[CHANNEL_USD_SEED, channel.as_ref()]),
                fees: pda(&[FEES_SEED]),
                markup: pda(&[MARKUP_SEED]),
                token_program: anchor_spl::token::ID,
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .args(sasona::instruction::SweepChannel {})
    } else if command == "ask-close-channel" {
        let channel: Pubkey = args[2].parse().expect("channel address");
        request
            .accounts(sasona::accounts::AskToCloseChannel { payer: me, channel })
            .args(sasona::instruction::AskToCloseChannel {})
    } else if command == "close-channel" {
        let channel: Pubkey = args[2].parse().expect("channel address");
        let c: sasona::Channel = program.account(channel).expect("channel");
        request
            .accounts(sasona::accounts::CloseChannel {
                closer: me,
                payer: c.payer,
                payer_usd: get_associated_token_address(&c.payer, &USD_MINT),
                channel,
                channel_usd: pda(&[CHANNEL_USD_SEED, channel.as_ref()]),
                fees: pda(&[FEES_SEED]),
                markup: pda(&[MARKUP_SEED]),
                token_program: anchor_spl::token::ID,
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .args(sasona::instruction::CloseChannel {})
    } else if command == "settle-markup" {
        request
            .accounts(sasona::accounts::SettleMarkup {
                caller: me,
                pool: pda(&[POOL_SEED]),
                coin_mint: coin,
                pool_usd: pda(&[POOL_USD_SEED]),
                pool_coin: pda(&[POOL_COIN_SEED]),
                fees: pda(&[FEES_SEED]),
                markup: pda(&[MARKUP_SEED]),
                network: pda(&[NETWORK_SEED]),
                token_program: anchor_spl::token::ID,
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .args(sasona::instruction::SettleMarkup {})
    } else if command == "settle" {
        request
            .accounts(sasona::accounts::SettleEntryFees {
                caller: me,
                pool: pda(&[POOL_SEED]),
                coin_mint: coin,
                pool_usd: pda(&[POOL_USD_SEED]),
                pool_coin: pda(&[POOL_COIN_SEED]),
                fees: pda(&[FEES_SEED]),
                markup: pda(&[MARKUP_SEED]),
                network: pda(&[NETWORK_SEED]),
                token_program: anchor_spl::token::ID,
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .args(sasona::instruction::SettleEntryFees {})
    } else {
        request
            .accounts(sasona::accounts::Deposit {
                depositor: me,
                pool: pda(&[POOL_SEED]),
                coin_mint: coin,
                usd_mint: USD_MINT,
                pool_usd: pda(&[POOL_USD_SEED]),
                pool_coin: pda(&[POOL_COIN_SEED]),
                fees: pda(&[FEES_SEED]),
                depositor_usd: get_associated_token_address(&me, &USD_MINT),
                depositor_coin: get_associated_token_address(&me, &coin),
                cover: pda(&[COVER_SEED]),
                cover_vault: pda(&[COVER_VAULT_SEED]),
                legacy_vault: pda(&[VAULT_SEED, me.as_ref()]),
                guarantee: pda(&[GUARANTEE_SEED, me.as_ref()]),
                token_program: anchor_spl::token::ID,
                associated_token_program: anchor_spl::associated_token::ID,
                system_program: anchor_client::anchor_lang::system_program::ID,
            })
            .args(sasona::instruction::Deposit { amount })
    };
    let sig = request.send().unwrap_or_else(|e| panic!("{command} failed: {e}"));
    println!("{sig}");
}
