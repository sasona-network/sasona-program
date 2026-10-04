//! Send Sasona instructions to devnet.
//!
//!     sasona open <dollars> --keypair <path>
//!     sasona deposit <dollars> --keypair <path>
//!     sasona fee <dollars of markup> --keypair <path>
//!     sasona settle --keypair <path>
//!     sasona depth <dollars> --keypair <path>
//!     sasona join <owner> --keypair <path>
//!     sasona claim <dollars> <to dollar account> --keypair <judge key>
//!     sasona ask-back <shares> --keypair <path>
//!     sasona release --keypair <path>
//!
//! The keypair signs and pays. It must hold the devnet dollar in its
//! associated token account. Prints the transaction signature.

use std::rc::Rc;

use anchor_client::anchor_lang::prelude::Pubkey;
use anchor_client::{Client, Cluster, CommitmentConfig};
use anchor_spl::associated_token::get_associated_token_address;
use sasona::{
    COIN_SEED, COVER_SEED, COVER_VAULT_SEED, EXIT_SEED, FEES_SEED, GUARANTEE_SEED, NETWORK_SEED, POOL_COIN_SEED,
    POOL_SEED, POOL_USD_SEED, USD_MINT, VAULT_SEED,
};
use solana_keypair::read_keypair_file;
use solana_signer::Signer;

const USAGE: &str = "usage: sasona <open|deposit|fee|depth> <dollars> --keypair <path>
       sasona settle|release --keypair <path>
       sasona join <owner> --keypair <path>
       sasona claim <dollars> <to dollar account> --keypair <judge key> [--reference <text>]
       sasona ask-back <shares> --keypair <path>
       (all take [--url <rpc>])";

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn pda(seeds: &[&[u8]]) -> Pubkey {
    Pubkey::find_program_address(seeds, &sasona::ID).0
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).cloned().unwrap_or_default();
    if !["open", "deposit", "fee", "settle", "depth", "join", "claim", "ask-back", "release"].contains(&command.as_str()) {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }
    let dollars: f64 = if ["settle", "join", "release"].contains(&command.as_str()) { 0.0 } else { args.get(2).and_then(|s| s.parse().ok()).expect(USAGE) };
    let amount = (dollars * 1_000_000.0).round() as u64;
    let keypair = read_keypair_file(arg(&args, "--keypair").expect(USAGE)).expect("cannot read keypair");
    let cluster = match arg(&args, "--url") {
        Some(url) => Cluster::Custom(url.clone(), url.replace("https", "wss")),
        None => Cluster::Devnet,
    };

    let me = keypair.pubkey();
    let client = Client::new_with_options(cluster, Rc::new(keypair), CommitmentConfig::finalized());
    let program = client.program(sasona::ID).expect("program client");
    let coin = pda(&[COIN_SEED]);

    let request = program.request();
    let request = if command == "open" {
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
    } else if command == "claim" {
        let to: Pubkey = args[3].parse().expect("the dollar account to pay");
        let mut reference = [0u8; 32];
        if let Some(r) = arg(&args, "--reference") {
            let b = r.as_bytes();
            reference[..b.len().min(32)].copy_from_slice(&b[..b.len().min(32)]);
        }
        request
            .accounts(sasona::accounts::Claim {
                judge: me,
                pool: pda(&[POOL_SEED]),
                coin_mint: coin,
                usd_mint: USD_MINT,
                pool_usd: pda(&[POOL_USD_SEED]),
                pool_coin: pda(&[POOL_COIN_SEED]),
                fees: pda(&[FEES_SEED]),
                cover: pda(&[COVER_SEED]),
                cover_vault: pda(&[COVER_VAULT_SEED]),
                claimant_usd: to,
                token_program: anchor_spl::token::ID,
            })
            .args(sasona::instruction::Claim { dollars: amount, reference })
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
    } else if command == "settle" {
        request
            .accounts(sasona::accounts::SettleEntryFees {
                caller: me,
                pool: pda(&[POOL_SEED]),
                coin_mint: coin,
                pool_usd: pda(&[POOL_USD_SEED]),
                pool_coin: pda(&[POOL_COIN_SEED]),
                fees: pda(&[FEES_SEED]),
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
