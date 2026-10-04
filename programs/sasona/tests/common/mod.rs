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
    POOL_COIN_SEED, POOL_SEED, POOL_USD_SEED, USD_MINT, VAULT_SEED, COVER_SEED, COVER_VAULT_SEED, EXIT_SEED,
    Cover, Exit, JUDGE, MAX_SHARES_PER_COIN, NOTICE_SECONDS, coins_for_shares, shares_for,
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
    world_with(true)
}

/// Signatures are not checked, so a test can act as the judge, whose key it
/// does not hold. Only the claim tests use this: everywhere else a missing
/// signature is part of what is being tested.
pub fn world_unverified() -> World {
    world_with(false)
}

pub fn world_with(sigverify: bool) -> World {
    let mut svm = LiteSVM::new().with_sigverify(sigverify);
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
        cover: key(pda(&[COVER_SEED])),
        cover_vault: key(pda(&[COVER_VAULT_SEED])),
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
        cover: key(pda(&[COVER_SEED])),
        cover_vault: key(pda(&[COVER_VAULT_SEED])),
        legacy_vault: key(pda(&[VAULT_SEED, who.as_ref()])),
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
    if svm.get_account(&pda(&[COVER_SEED])).is_some() {
        let c: Cover = read(svm, pda(&[COVER_SEED]));
        assert!(token_balance(svm, pda(&[COVER_VAULT_SEED])) >= c.coins, "cover");
    }
}

pub fn add_depth_ix(who: Address, usd_mint: Address, from: Address, amount: u64) -> Instruction {
    let accounts = sasona::accounts::AddDepth {
        giver: key(who),
        pool: key(pda(&[POOL_SEED])),
        coin_mint: key(pda(&[COIN_SEED])),
        usd_mint: key(usd_mint),
        pool_usd: key(pda(&[POOL_USD_SEED])),
        fees: key(pda(&[FEES_SEED])),
        giver_usd: key(from),
        token_program: anchor_spl::token::ID,
    }
    .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::AddDepth { amount }.data() }
}

pub fn try_add_depth(svm: &mut LiteSVM, who: &Keypair, from: Address, amount: u64) -> Result<(), String> {
    svm.expire_blockhash();
    send(svm, add_depth_ix(who.pubkey(), usd(), from, amount), &[who])
}

/// Shares a person holds in the cover.
pub fn shares_of(svm: &LiteSVM, who: Address) -> u64 {
    let g: Guarantee = read(svm, pda(&[GUARANTEE_SEED, who.as_ref()]));
    g.shares
}

pub fn cover(svm: &LiteSVM) -> Cover {
    read(svm, pda(&[COVER_SEED]))
}

pub fn claim_ix(judge: Address, to: Address, dollars: u64) -> Instruction {
    let accounts = sasona::accounts::Claim {
        judge: key(judge),
        pool: key(pda(&[POOL_SEED])),
        coin_mint: key(pda(&[COIN_SEED])),
        usd_mint: key(usd()),
        pool_usd: key(pda(&[POOL_USD_SEED])),
        pool_coin: key(pda(&[POOL_COIN_SEED])),
        fees: key(pda(&[FEES_SEED])),
        cover: key(pda(&[COVER_SEED])),
        cover_vault: key(pda(&[COVER_VAULT_SEED])),
        claimant_usd: key(to),
        token_program: anchor_spl::token::ID,
    }
    .to_account_metas(None);
    let data = sasona::instruction::Claim { dollars, reference: [7u8; 32] }.data();
    Instruction { program_id: program_id(), accounts: metas(accounts), data }
}

/// Send as the judge. Only works in `world_unverified`.
pub fn claim_as_judge(svm: &mut LiteSVM, to: Address, dollars: u64) -> Result<(), String> {
    let judge = addr(JUDGE);
    if svm.get_account(&judge).map(|a| a.lamports).unwrap_or(0) == 0 {
        svm.airdrop(&judge, 10_000_000_000).unwrap();
    }
    svm.expire_blockhash();
    let msg = solana_message::Message::new_with_blockhash(&[claim_ix(judge, to, dollars)], Some(&judge), &svm.latest_blockhash());
    let tx = Transaction::new_unsigned(msg);
    svm.send_transaction(tx).map(|_| ()).map_err(|e| format!("{:?}\n{}", e.err, e.meta.logs.join("\n")))
}

pub fn request_ix(owner: Address, shares: u64) -> Instruction {
    let accounts = sasona::accounts::RequestRelease {
        owner: key(owner),
        guarantee: key(pda(&[GUARANTEE_SEED, owner.as_ref()])),
        exit: key(pda(&[EXIT_SEED, owner.as_ref()])),
        legacy_vault: key(pda(&[VAULT_SEED, owner.as_ref()])),
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::RequestRelease { shares }.data() }
}

pub fn release_ix(owner: Address) -> Instruction {
    let coin = pda(&[COIN_SEED]);
    let accounts = sasona::accounts::Release {
        owner: key(owner),
        pool: key(pda(&[POOL_SEED])),
        coin_mint: key(coin),
        cover: key(pda(&[COVER_SEED])),
        cover_vault: key(pda(&[COVER_VAULT_SEED])),
        exit: key(pda(&[EXIT_SEED, owner.as_ref()])),
        owner_coin: key(ata(owner, coin)),
        token_program: anchor_spl::token::ID,
        associated_token_program: anchor_spl::associated_token::ID,
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::Release {}.data() }
}

pub fn join_ix(caller: Address, owner: Address) -> Instruction {
    let accounts = sasona::accounts::JoinCover {
        caller: key(caller),
        pool: key(pda(&[POOL_SEED])),
        coin_mint: key(pda(&[COIN_SEED])),
        cover: key(pda(&[COVER_SEED])),
        cover_vault: key(pda(&[COVER_VAULT_SEED])),
        owner: key(owner),
        guarantee: key(pda(&[GUARANTEE_SEED, owner.as_ref()])),
        legacy_vault: key(pda(&[VAULT_SEED, owner.as_ref()])),
        token_program: anchor_spl::token::ID,
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::JoinCover {}.data() }
}

pub fn try_ix(svm: &mut LiteSVM, ix: Instruction, who: &Keypair) -> Result<(), String> {
    svm.expire_blockhash();
    send(svm, ix, &[who])
}

pub fn days_pass(svm: &mut LiteSVM, days: i64) {
    let mut c: solana_clock::Clock = svm.get_sysvar();
    c.unix_timestamp += days * 24 * 60 * 60;
    svm.set_sysvar(&c);
}

pub fn round_address(fingerprint: [u8; 32]) -> Address {
    pda(&[sasona::ROUND_SEED, fingerprint.as_ref()])
}

pub fn open_round_ix(opener: Address, fingerprint: [u8; 32], size: u32, count: u16, seed_hash: [u8; 32]) -> Instruction {
    let accounts = sasona::accounts::OpenRound {
        opener: key(opener),
        round: key(round_address(fingerprint)),
        members: key(members_address()),
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    let data = sasona::instruction::OpenRound { pool_fingerprint: fingerprint, pool_size: size, count, seed_hash }.data();
    Instruction { program_id: program_id(), accounts: metas(accounts), data }
}

pub fn reveal_ix(round: Address, opener: Address, seed: [u8; 32]) -> Instruction {
    let accounts = sasona::accounts::RevealRound {
        round: key(round),
        opener: key(opener),
        slot_hashes: sasona::SLOT_HASHES_ID,
    }
    .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::RevealRound { seed }.data() }
}

pub fn withheld_ix(round: Address) -> Instruction {
    let accounts = sasona::accounts::MarkWithheld { round: key(round), slot_hashes: sasona::SLOT_HASHES_ID }
        .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::MarkWithheld {}.data() }
}

/// Move the clock to `slot`, and make SlotHashes hold `entries`, newest
/// first, as (slot, hash).
pub fn at_slot(svm: &mut LiteSVM, slot: u64, entries: &[(u64, [u8; 32])]) {
    svm.warp_to_slot(slot);
    let mut data = (entries.len() as u64).to_le_bytes().to_vec();
    for (s, h) in entries {
        data.extend_from_slice(&s.to_le_bytes());
        data.extend_from_slice(h);
    }
    let sysvar_owner = Address::from_str_const("Sysvar1111111111111111111111111111111111111");
    svm.set_account(addr(sasona::SLOT_HASHES_ID), Account { lamports: 1_000_000_000, data, owner: sysvar_owner, executable: false, rent_epoch: 0 })
        .unwrap();
}

/// The hash of a slot, made up for tests.
pub fn slot_hash(slot: u64) -> [u8; 32] {
    let mut h = [0u8; 32];
    h[..8].copy_from_slice(&slot.to_le_bytes());
    h[31] = 0xAB;
    h
}

/// SlotHashes as it stands at `now`: the 512 slots before it, newest first,
/// leaving out any slot in `skipped`.
pub fn recent(now: u64, skipped: &[u64]) -> Vec<(u64, [u8; 32])> {
    (now.saturating_sub(512)..now).rev().filter(|s| !skipped.contains(s)).map(|s| (s, slot_hash(s))).collect()
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    solana_sha256_hasher::hashv(&[data]).to_bytes()
}

pub fn reading_address(round: Address, endpoint: &str) -> Address {
    pda(&[sasona::READING_SEED, round.as_ref(), &sha256(endpoint.as_bytes())])
}

/// A commitment by `reader`, with the membership the draw gives them and the
/// memberships it passed over before theirs. If they were not drawn, it names
/// a membership of theirs anyway, or the drawn one if they hold none, so the
/// program is what refuses it.
pub fn commit_reading_ix(svm: &LiteSVM, reader: Address, round: Address, endpoint: &str, question_hash: [u8; 32]) -> Instruction {
    let (member, skipped) = reader_accounts(svm, reader, round, endpoint, None);
    let accounts = sasona::accounts::CommitReading {
        reader: key(reader),
        round: key(round),
        member: key(member),
        reading: key(reading_address(round, endpoint)),
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    let mut accounts = metas(accounts);
    accounts.extend(skipped.into_iter().map(|a| AccountMeta::new_readonly(a, false)));
    let data = sasona::instruction::CommitReading {
        endpoint_hash: sha256(endpoint.as_bytes()),
        endpoint: endpoint.to_string(),
        question_hash,
    }
    .data();
    Instruction { program_id: program_id(), accounts, data }
}

pub fn reveal_reading_ix(reader: Address, reading: Address, nonce: [u8; 16], reply_hash: [u8; 32], verdict: u8) -> Instruction {
    let accounts = sasona::accounts::RevealReading {
        reader: key(reader),
        reading: key(reading),
        used_nonce: key(pda(&[sasona::NONCE_SEED, &nonce])),
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    let data = sasona::instruction::RevealReading { nonce, reply_hash, verdict }.data();
    Instruction { program_id: program_id(), accounts: metas(accounts), data }
}

pub fn commit_second_ix(svm: &LiteSVM, reader: Address, round: Address, endpoint: &str, question_hash: [u8; 32], first: Address) -> Instruction {
    let reading = reading_address(round, endpoint);
    let first_reader = svm
        .get_account(&first)
        .and_then(|a| sasona::Reading::try_deserialize(&mut a.data.as_slice()).ok())
        .map(|r| addr(r.reader));
    let (member, skipped) = reader_accounts(svm, reader, round, endpoint, first_reader);
    let accounts = sasona::accounts::CommitSecondReading {
        reader: key(reader),
        round: key(round),
        member: key(member),
        reading: key(reading),
        first: key(first),
        pair: key(pda(&[sasona::PAIR_SEED, reading.as_ref()])),
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    let mut accounts = metas(accounts);
    accounts.extend(skipped.into_iter().map(|a| AccountMeta::new_readonly(a, false)));
    let data = sasona::instruction::CommitSecondReading {
        endpoint_hash: sha256(endpoint.as_bytes()),
        endpoint: endpoint.to_string(),
        question_hash,
    }
    .data();
    Instruction { program_id: program_id(), accounts, data }
}

pub fn settle_pair_ix(first: Address, second: Address) -> Instruction {
    let accounts = sasona::accounts::SettlePair {
        pair: key(pda(&[sasona::PAIR_SEED, second.as_ref()])),
        first: key(first),
        second: key(second),
    }
    .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::SettlePair {}.data() }
}

pub fn lapsed_ix(reading: Address) -> Instruction {
    let accounts = sasona::accounts::MarkLapsed { reading: key(reading) }.to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::MarkLapsed {}.data() }
}

/// Build an instruction from the current state, then send it.
pub fn try_with(svm: &mut LiteSVM, make: impl FnOnce(&LiteSVM) -> Instruction, who: &Keypair) -> Result<(), String> {
    let ix = make(svm);
    try_ix(svm, ix, who)
}

// ------------------------------------------------------------------ members

pub fn members_address() -> Address {
    pda(&[sasona::MEMBERS_SEED])
}

pub fn member_address(number: u32) -> Address {
    pda(&[sasona::MEMBER_SEED, &number.to_le_bytes()])
}

pub fn member(svm: &LiteSVM, number: u32) -> sasona::Member {
    read(svm, member_address(number))
}

pub fn member_count(svm: &LiteSVM) -> u32 {
    svm.get_account(&members_address())
        .and_then(|a| sasona::Members::try_deserialize(&mut a.data.as_slice()).ok())
        .map(|m| m.count)
        .unwrap_or(0)
}

pub fn join_members_ix(owner: Address, number: u32) -> Instruction {
    let coin = pda(&[COIN_SEED]);
    let accounts = sasona::accounts::JoinMembers {
        owner: key(owner),
        pool: key(pda(&[POOL_SEED])),
        coin_mint: key(coin),
        owner_coin: key(ata(owner, coin)),
        members: key(members_address()),
        member: key(member_address(number)),
        stakes: key(pda(&[sasona::STAKES_SEED])),
        held: key(pda(&[sasona::HELD_SEED])),
        token_program: anchor_spl::token::ID,
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::JoinMembers {}.data() }
}

/// Take a membership for `who`, who must hold the coins. Returns its number.
pub fn join(svm: &mut LiteSVM, who: &Keypair) -> u32 {
    let number = member_count(svm) + 1;
    try_ix(svm, join_members_ix(who.pubkey(), number), who).expect("join");
    number
}

/// Someone new who deposited $100, which leaves them coins for a stake, and
/// took a membership.
pub fn new_member(svm: &mut LiteSVM) -> (Keypair, u32) {
    let (k, usd) = newcomer(svm, 100);
    try_deposit(svm, &k, usd, 100 * DOLLAR).expect("deposit");
    let n = join(svm, &k);
    (k, n)
}

/// The pool opened by the depositor, who holds membership 1.
pub fn world_with_member() -> World {
    let mut w = world();
    open(&mut w, 1_000 * DOLLAR);
    let d = w.depositor.insecure_clone();
    join(&mut w.svm, &d);
    w
}

pub fn ask_to_leave_ix(owner: Address, number: u32) -> Instruction {
    let accounts = sasona::accounts::AskToLeave { owner: key(owner), member: key(member_address(number)) }.to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::AskToLeave {}.data() }
}

pub fn leave_ix(owner: Address, number: u32) -> Instruction {
    let coin = pda(&[COIN_SEED]);
    let accounts = sasona::accounts::Leave {
        owner: key(owner),
        pool: key(pda(&[POOL_SEED])),
        coin_mint: key(coin),
        member: key(member_address(number)),
        stakes: key(pda(&[sasona::STAKES_SEED])),
        owner_coin: key(ata(owner, coin)),
        token_program: anchor_spl::token::ID,
    }
    .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::Leave {}.data() }
}

/// SPEC.md 4.3, followed against the accounts: the membership drawn for
/// `endpoint` in `round`, and those passed over before it.
pub fn drawn_for(svm: &LiteSVM, round: Address, endpoint: &str, first_reader: Option<Address>) -> (Option<u32>, Vec<u32>) {
    let r: sasona::Round = read(svm, round);
    let mut skipped = vec![];
    if r.members == 0 {
        return (None, skipped);
    }
    let h = sha256(endpoint.as_bytes());
    for a in 0..sasona::MAX_READER_ATTEMPTS {
        let k = sasona::reader_number(&r.final_seed, &h, a, r.members);
        let m = member(svm, k);
        if m.state == sasona::MEMBER_ACTIVE && Some(addr(m.owner)) != first_reader {
            return (Some(k), skipped);
        }
        skipped.push(k);
    }
    (None, skipped)
}

fn reader_accounts(svm: &LiteSVM, reader: Address, round: Address, endpoint: &str, first_reader: Option<Address>) -> (Address, Vec<Address>) {
    let (drawn, skipped) = drawn_for(svm, round, endpoint, first_reader);
    if let Some(k) = drawn {
        if addr(member(svm, k).owner) == reader {
            return (member_address(k), skipped.into_iter().map(member_address).collect());
        }
    }
    let own = (1..=member_count(svm)).find(|&n| addr(member(svm, n).owner) == reader);
    (member_address(own.or(drawn).unwrap_or(1)), vec![])
}

// --------------------------------------------------------------- challenges

pub fn challenge_address(reading: Address) -> Address {
    pda(&[sasona::CHALLENGE_SEED, reading.as_ref()])
}

pub fn evidence_address(reading: Address) -> Address {
    pda(&[sasona::EVIDENCE_SEED, reading.as_ref()])
}

fn member_of(svm: &LiteSVM, reading: Address) -> Address {
    let r: sasona::Reading = read(svm, reading);
    member_address(r.member)
}

pub fn challenge_ix(svm: &LiteSVM, challenger: Address, reading: Address) -> Instruction {
    let accounts = sasona::accounts::ChallengeReading {
        challenger: key(challenger),
        reading: key(reading),
        member: key(member_of(svm, reading)),
        challenge: key(challenge_address(reading)),
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::Challenge {}.data() }
}

pub fn open_evidence_ix(reader: Address, reading: Address, len: u32) -> Instruction {
    let accounts = sasona::accounts::OpenEvidence {
        reader: key(reader),
        reading: key(reading),
        evidence: key(evidence_address(reading)),
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::OpenEvidence { len }.data() }
}

pub fn write_evidence_ix(reader: Address, reading: Address, offset: u32, bytes: &[u8]) -> Instruction {
    let accounts = sasona::accounts::WriteEvidence {
        reader: key(reader),
        reading: key(reading),
        evidence: key(evidence_address(reading)),
    }
    .to_account_metas(None);
    let data = sasona::instruction::WriteEvidence { offset, bytes: bytes.to_vec() }.data();
    Instruction { program_id: program_id(), accounts: metas(accounts), data }
}

/// Put a whole reply on chain, in pieces small enough for a transaction.
pub fn put_evidence(svm: &mut LiteSVM, reader: &Keypair, reading: Address, reply: &[u8]) -> Result<(), String> {
    try_ix(svm, open_evidence_ix(reader.pubkey(), reading, reply.len() as u32), reader)?;
    for (i, chunk) in reply.chunks(800).enumerate() {
        try_ix(svm, write_evidence_ix(reader.pubkey(), reading, (i * 800) as u32, chunk), reader)?;
    }
    Ok(())
}

pub fn answer_ix(svm: &LiteSVM, reading: Address, nonce: [u8; 16]) -> Instruction {
    let r: sasona::Reading = read(svm, reading);
    let accounts = sasona::accounts::AnswerChallenge {
        challenge: key(challenge_address(reading)),
        reading: key(reading),
        evidence: key(evidence_address(reading)),
        used_nonce: key(pda(&[sasona::NONCE_SEED, &nonce])),
        member: key(member_address(r.member)),
        reader: r.reader,
    }
    .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::AnswerChallenge { nonce }.data() }
}

pub fn uphold_ix(svm: &LiteSVM, reading: Address) -> Instruction {
    let c: sasona::Challenge = read(svm, challenge_address(reading));
    let coin = pda(&[COIN_SEED]);
    let accounts = sasona::accounts::UpholdChallenge {
        challenge: key(challenge_address(reading)),
        reading: key(reading),
        member: key(member_of(svm, reading)),
        pool: key(pda(&[POOL_SEED])),
        coin_mint: key(coin),
        stakes: key(pda(&[sasona::STAKES_SEED])),
        held: key(pda(&[sasona::HELD_SEED])),
        challenger: c.challenger,
        challenger_coin: key(ata(addr(c.challenger), coin)),
        token_program: anchor_spl::token::ID,
    }
    .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::UpholdChallenge {}.data() }
}
