//! Helpers shared by the test files.
#![allow(dead_code, unused_imports)]

pub use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
pub use litesvm::LiteSVM;
pub use solana_account::Account;
pub use solana_address::Address;
pub use solana_instruction::{AccountMeta, Instruction};
pub use solana_keypair::Keypair;
pub use solana_program_pack::Pack;
pub use solana_signer::Signer;
pub use solana_transaction::Transaction;
pub use spl_token_interface::state::{Account as TokenAccount, AccountState, Mint};

pub use sasona::{at_price, burn_of, coins_out, Fee, NETWORK_SEED, BURN_BPS, 
    Guarantee, Pool, Slices, COIN_DECIMALS, COIN_SEED, FEES_SEED, GUARANTEE_SEED, OPENING_COINS_PER_USD,
    POOL_COIN_SEED, POOL_SEED, POOL_USD_SEED, USD_MINT, VAULT_SEED,
};

pub const DOLLAR: u64 = 1_000_000;
pub const PRICE: u64 = OPENING_COINS_PER_USD;

pub fn addr(p: anchor_lang::prelude::Pubkey) -> Address {
    Address::new_from_array(p.to_bytes())
}

pub fn key(a: Address) -> anchor_lang::prelude::Pubkey {
    anchor_lang::prelude::Pubkey::new_from_array(a.to_bytes())
}

pub fn program_id() -> Address {
    addr(sasona::ID)
}

pub fn pda(seeds: &[&[u8]]) -> Address {
    addr(anchor_lang::prelude::Pubkey::find_program_address(seeds, &sasona::ID).0)
}

pub fn token_program() -> Address {
    addr(anchor_spl::token::ID)
}

pub fn usd() -> Address {
    addr(USD_MINT)
}

pub fn ata(owner: Address, mint: Address) -> Address {
    addr(
        anchor_lang::prelude::Pubkey::find_program_address(
            &[owner.as_ref(), anchor_spl::token::ID.as_ref(), mint.as_ref()],
            &anchor_spl::associated_token::ID,
        )
        .0,
    )
}

pub struct World {
    pub svm: LiteSVM,
    pub depositor: Keypair,
    pub depositor_usd: Address,
}

pub fn put_mint(svm: &mut LiteSVM, at: Address, authority: Address, decimals: u8) {
    let mut data = vec![0u8; Mint::LEN];
    Mint {
        mint_authority: Some(authority).into(),
        supply: 0,
        decimals,
        is_initialized: true,
        freeze_authority: None.into(),
    }
    .pack_into_slice(&mut data);
    svm.set_account(at, Account { lamports: 1_000_000_000, data, owner: token_program(), executable: false, rent_epoch: 0 })
        .unwrap();
}

pub fn put_token_account(svm: &mut LiteSVM, at: Address, mint: Address, owner: Address, amount: u64) {
    let mut data = vec![0u8; TokenAccount::LEN];
    TokenAccount {
        mint,
        owner,
        amount,
        delegate: None.into(),
        state: AccountState::Initialized,
        is_native: None.into(),
        delegated_amount: 0,
        close_authority: None.into(),
    }
    .pack_into_slice(&mut data);
    svm.set_account(at, Account { lamports: 1_000_000_000, data, owner: token_program(), executable: false, rent_epoch: 0 })
        .unwrap();
}

pub fn world() -> World {
    let mut svm = LiteSVM::new();
    let so = std::env::var("SASONA_SO").expect("run through scripts/build.sh, which sets SASONA_SO");
    svm.add_program_from_file(program_id(), so).unwrap();

    let depositor = Keypair::new();
    svm.airdrop(&depositor.pubkey(), 100_000_000_000).unwrap();

    // The dollar lives at the address the program expects, as it does on devnet.
    put_mint(&mut svm, usd(), Address::new_unique(), 6);
    let depositor_usd = Address::new_unique();
    put_token_account(&mut svm, depositor_usd, usd(), depositor.pubkey(), 10_000 * DOLLAR);

    World { svm, depositor, depositor_usd }
}

pub fn open_ix(w: &World, usd_mint: Address, depositor_usd: Address, amount: u64) -> Instruction {
    let d = w.depositor.pubkey();
    let coin = pda(&[COIN_SEED]);
    let accounts = sasona::accounts::Open {
        depositor: key(d),
        pool: key(pda(&[POOL_SEED])),
        coin_mint: key(coin),
        usd_mint: key(usd_mint),
        pool_usd: key(pda(&[POOL_USD_SEED])),
        pool_coin: key(pda(&[POOL_COIN_SEED])),
        fees: key(pda(&[FEES_SEED])),
        depositor_usd: key(depositor_usd),
        depositor_coin: key(ata(d, coin)),
        guarantee_vault: key(pda(&[VAULT_SEED, d.as_ref()])),
        guarantee: key(pda(&[GUARANTEE_SEED, d.as_ref()])),
        token_program: anchor_spl::token::ID,
        associated_token_program: anchor_spl::associated_token::ID,
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None)
    .into_iter()
    .map(|m| AccountMeta { pubkey: addr(m.pubkey), is_signer: m.is_signer, is_writable: m.is_writable })
    .collect();
    let data = sasona::instruction::Open { amount }.data();
    Instruction { program_id: program_id(), accounts, data }
}

pub fn send(svm: &mut LiteSVM, ix: Instruction, signers: &[&Keypair]) -> Result<(), String> {
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&signers[0].pubkey()), signers, svm.latest_blockhash());
    svm.send_transaction(tx).map(|_| ()).map_err(|e| format!("{:?}\n{}", e.err, e.meta.logs.join("\n")))
}

pub fn token_balance(svm: &LiteSVM, at: Address) -> u64 {
    TokenAccount::unpack(&svm.get_account(&at).unwrap().data).unwrap().amount
}

pub fn mint_state(svm: &LiteSVM, at: Address) -> Mint {
    Mint::unpack(&svm.get_account(&at).unwrap().data).unwrap()
}

pub fn read<T: AccountDeserialize>(svm: &LiteSVM, at: Address) -> T {
    T::try_deserialize(&mut svm.get_account(&at).unwrap().data.as_slice()).unwrap()
}

pub fn try_open(w: &mut World, usd_mint: Address, depositor_usd: Address, amount: u64) -> Result<(), String> {
    let ix = open_ix(w, usd_mint, depositor_usd, amount);
    let d = w.depositor.insecure_clone();
    send(&mut w.svm, ix, &[&d])
}

pub fn open(w: &mut World, amount: u64) {
    let from = w.depositor_usd;
    try_open(w, usd(), from, amount).expect("open should succeed");
}


/// Someone new, with SOL for fees and `dollars` of the devnet dollar.
pub fn newcomer(svm: &mut LiteSVM, dollars: u64) -> (Keypair, Address) {
    let k = Keypair::new();
    svm.airdrop(&k.pubkey(), 10_000_000_000).unwrap();
    let account = Address::new_unique();
    put_token_account(svm, account, usd(), k.pubkey(), dollars * DOLLAR);
    (k, account)
}

pub fn deposit_ix(who: Address, usd_mint: Address, from: Address, amount: u64) -> Instruction {
    let coin = pda(&[COIN_SEED]);
    let accounts = sasona::accounts::Deposit {
        depositor: key(who),
        pool: key(pda(&[POOL_SEED])),
        coin_mint: key(coin),
        usd_mint: key(usd_mint),
        pool_usd: key(pda(&[POOL_USD_SEED])),
        pool_coin: key(pda(&[POOL_COIN_SEED])),
        fees: key(pda(&[FEES_SEED])),
        depositor_usd: key(from),
        depositor_coin: key(ata(who, coin)),
        guarantee_vault: key(pda(&[VAULT_SEED, who.as_ref()])),
        guarantee: key(pda(&[GUARANTEE_SEED, who.as_ref()])),
        token_program: anchor_spl::token::ID,
        associated_token_program: anchor_spl::associated_token::ID,
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None)
    .into_iter()
    .map(|m| AccountMeta { pubkey: addr(m.pubkey), is_signer: m.is_signer, is_writable: m.is_writable })
    .collect();
    let data = sasona::instruction::Deposit { amount }.data();
    Instruction { program_id: program_id(), accounts, data }
}

pub fn try_deposit(svm: &mut LiteSVM, who: &Keypair, from: Address, amount: u64) -> Result<(), String> {
    svm.expire_blockhash();
    send(svm, deposit_ix(who.pubkey(), usd(), from, amount), &[who])
}

pub fn pool(svm: &LiteSVM) -> Pool {
    read(svm, pda(&[POOL_SEED]))
}

fn metas(accounts: Vec<anchor_lang::prelude::AccountMeta>) -> Vec<AccountMeta> {
    accounts
        .into_iter()
        .map(|m| AccountMeta { pubkey: addr(m.pubkey), is_signer: m.is_signer, is_writable: m.is_writable })
        .collect()
}

pub fn pay_fee_ix(who: Address, usd_mint: Address, from: Address, markup: u64) -> Instruction {
    let accounts = sasona::accounts::PayFee {
        payer: key(who),
        pool: key(pda(&[POOL_SEED])),
        coin_mint: key(pda(&[COIN_SEED])),
        usd_mint: key(usd_mint),
        pool_usd: key(pda(&[POOL_USD_SEED])),
        pool_coin: key(pda(&[POOL_COIN_SEED])),
        payer_usd: key(from),
        fees: key(pda(&[FEES_SEED])),
        network: key(pda(&[NETWORK_SEED])),
        token_program: anchor_spl::token::ID,
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::PayFee { markup }.data() }
}

pub fn settle_ix(caller: Address) -> Instruction {
    let accounts = sasona::accounts::SettleEntryFees {
        caller: key(caller),
        pool: key(pda(&[POOL_SEED])),
        coin_mint: key(pda(&[COIN_SEED])),
        pool_usd: key(pda(&[POOL_USD_SEED])),
        pool_coin: key(pda(&[POOL_COIN_SEED])),
        fees: key(pda(&[FEES_SEED])),
        network: key(pda(&[NETWORK_SEED])),
        token_program: anchor_spl::token::ID,
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::SettleEntryFees {}.data() }
}

pub fn try_pay_fee(svm: &mut LiteSVM, who: &Keypair, from: Address, markup: u64) -> Result<(), String> {
    svm.expire_blockhash();
    send(svm, pay_fee_ix(who.pubkey(), usd(), from, markup), &[who])
}

pub fn try_settle(svm: &mut LiteSVM, who: &Keypair) -> Result<(), String> {
    svm.expire_blockhash();
    send(svm, settle_ix(who.pubkey()), &[who])
}

/// Everything the pool records must match what the token program holds.
pub fn assert_books_balance(svm: &LiteSVM) {
    let p = pool(svm);
    assert_eq!(mint_state(svm, pda(&[COIN_SEED])).supply, p.coin_reserve + p.outside, "supply");
    assert_eq!(token_balance(svm, pda(&[POOL_COIN_SEED])), p.coin_reserve, "pool coins");
    assert!(token_balance(svm, pda(&[POOL_USD_SEED])) >= p.usd_reserve, "pool dollars");
    assert!(token_balance(svm, pda(&[FEES_SEED])) >= p.fees_held, "fees");
}
