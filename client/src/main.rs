//! Send Sasona instructions to devnet.
//!
//!     sasona open <dollars> --keypair <path>
//!
//! The keypair signs and pays. It must hold the devnet dollar in its
//! associated token account. Prints the transaction signature.

use std::rc::Rc;

use anchor_client::{Client, Cluster, CommitmentConfig};
use solana_keypair::read_keypair_file;
use solana_signer::Signer;
use anchor_spl::associated_token::get_associated_token_address;
use sasona::{
    COIN_SEED, FEES_SEED, GUARANTEE_SEED, POOL_COIN_SEED, POOL_SEED, POOL_USD_SEED, USD_MINT, VAULT_SEED,
};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let usage = "usage: sasona open <dollars> --keypair <path> [--url <rpc>]";
    if args.get(1).map(String::as_str) != Some("open") {
        eprintln!("{usage}");
        std::process::exit(2);
    }
    let dollars: f64 = args.get(2).and_then(|s| s.parse().ok()).expect(usage);
    let amount = (dollars * 1_000_000.0).round() as u64;
    let keypair = read_keypair_file(arg(&args, "--keypair").expect(usage)).expect("cannot read keypair");
    let cluster = match arg(&args, "--url") {
        Some(url) => Cluster::Custom(url.clone(), url.replace("https", "wss")),
        None => Cluster::Devnet,
    };

    let me = keypair.pubkey();
    let client = Client::new_with_options(cluster, Rc::new(keypair), CommitmentConfig::finalized());
    let program = client.program(sasona::ID).expect("program client");
    let pda = |seeds: &[&[u8]]| anchor_client::anchor_lang::prelude::Pubkey::find_program_address(seeds, &sasona::ID).0;
    let coin = pda(&[COIN_SEED]);

    let sig = program
        .request()
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
            guarantee_vault: pda(&[VAULT_SEED, me.as_ref()]),
            guarantee: pda(&[GUARANTEE_SEED, me.as_ref()]),
            token_program: anchor_spl::token::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_client::anchor_lang::system_program::ID,
        })
        .args(sasona::instruction::Open { amount })
        .send()
        .expect("open failed");
    println!("{sig}");
}
