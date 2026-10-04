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
    Cover, Exit, MAX_SHARES_PER_COIN, NOTICE_SECONDS, coins_for_shares, shares_for,
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
    // The simulator's clock starts far ahead, and tests set slots in the
    // thousands. A chain's slots only go forward, so start at the beginning.
    svm.warp_to_slot(1);
    // And its clock starts at 1970. A chain's starts at today, so that a time
    // left at zero, never recorded, looks long past and not just now.
    let mut clock: solana_clock::Clock = svm.get_sysvar();
    clock.unix_timestamp = 1_790_000_000;
    svm.set_sysvar(&clock);
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
        members: key(members_address()),
        member: key(member),
        seat: key(seat_of(svm, member)),
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
    reveal_paid_ix(reader, reading, nonce, reply_hash, verdict, Address::default())
}

/// A reveal that also records where the service asks to be paid (7.1).
pub fn reveal_paid_ix(reader: Address, reading: Address, nonce: [u8; 16], reply_hash: [u8; 32], verdict: u8, pay_to: Address) -> Instruction {
    let accounts = sasona::accounts::RevealReading {
        reader: key(reader),
        reading: key(reading),
        used_nonce: key(pda(&[sasona::NONCE_SEED, &nonce])),
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    let data = sasona::instruction::RevealReading { nonce, reply_hash, verdict, pay_to: key(pay_to) }.data();
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
        members: key(members_address()),
        member: key(member),
        seat: key(seat_of(svm, member)),
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

pub fn members_state(svm: &LiteSVM) -> Option<sasona::Members> {
    svm.get_account(&members_address()).and_then(|a| sasona::Members::try_deserialize(&mut a.data.as_slice()).ok())
}

/// Memberships ever taken.
pub fn member_count(svm: &LiteSVM) -> u32 {
    members_state(svm).map(|m| m.count).unwrap_or(0)
}

/// Seats on the roster now.
pub fn seated(svm: &LiteSVM) -> u32 {
    members_state(svm).map(|m| m.seated).unwrap_or(0)
}

pub fn seat_address(k: u32) -> Address {
    pda(&[sasona::SEAT_SEED, &k.to_le_bytes()])
}

pub fn seat(svm: &LiteSVM, k: u32) -> sasona::Seat {
    read(svm, seat_address(k))
}

pub fn join_members_ix(owner: Address, number: u32, seat: u32) -> Instruction {
    let coin = pda(&[COIN_SEED]);
    let accounts = sasona::accounts::JoinMembers {
        owner: key(owner),
        pool: key(pda(&[POOL_SEED])),
        coin_mint: key(coin),
        owner_coin: key(ata(owner, coin)),
        members: key(members_address()),
        member: key(member_address(number)),
        seat: key(seat_address(seat)),
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
    let s = seated(svm) + 1;
    try_ix(svm, join_members_ix(who.pubkey(), number, s), who).expect("join");
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

/// The accounts that take membership `number` off the roster: its seat, and
/// unless it sits last, the last seat and the membership in it.
pub fn unseat_accounts(svm: &LiteSVM, number: u32) -> Vec<AccountMeta> {
    let k = member(svm, number).seat;
    let last = seated(svm);
    let mut out = vec![AccountMeta::new(seat_address(k), false)];
    if k != last && k != 0 {
        out.push(AccountMeta::new(seat_address(last), false));
        out.push(AccountMeta::new(member_address(seat(svm, last).member), false));
    }
    out
}

pub fn ask_to_leave_ix(svm: &LiteSVM, owner: Address, number: u32) -> Instruction {
    let accounts = sasona::accounts::AskToLeave {
        owner: key(owner),
        members: key(members_address()),
        member: key(member_address(number)),
    }
    .to_account_metas(None);
    let mut accounts = metas(accounts);
    accounts.extend(unseat_accounts(svm, number));
    Instruction { program_id: program_id(), accounts, data: sasona::instruction::AskToLeave {}.data() }
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
        book: key(book_address(number)),
        token_program: anchor_spl::token::ID,
    }
    .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::Leave {}.data() }
}

/// SPEC.md 4.3, followed against the accounts: the seat drawn for
/// `endpoint` in `round`, and the seats passed over that must be shown.
pub fn drawn_for(svm: &LiteSVM, round: Address, endpoint: &str, first_reader: Option<Address>) -> (Option<u32>, Vec<u32>) {
    let r: sasona::Round = read(svm, round);
    let mut skipped = vec![];
    if r.members == 0 {
        return (None, skipped);
    }
    let h = sha256(endpoint.as_bytes());
    let now = seated(svm);
    for a in 0..sasona::MAX_READER_ATTEMPTS {
        let k = sasona::reader_number(&r.final_seed, &h, a, r.members);
        if k > now {
            continue;
        }
        let s = seat(svm, k);
        if s.since >= r.commit_slot || Some(addr(s.owner)) == first_reader {
            skipped.push(k);
            continue;
        }
        return (Some(k), skipped);
    }
    (None, skipped)
}

fn reader_accounts(svm: &LiteSVM, reader: Address, round: Address, endpoint: &str, first_reader: Option<Address>) -> (Address, Vec<Address>) {
    let (drawn, skipped) = drawn_for(svm, round, endpoint, first_reader);
    if let Some(k) = drawn {
        let s = seat(svm, k);
        if addr(s.owner) == reader {
            return (member_address(s.member), skipped.into_iter().map(seat_address).collect());
        }
    }
    let owned: Vec<u32> = (1..=member_count(svm)).filter(|&n| addr(member(svm, n).owner) == reader).collect();
    let own = owned.iter().copied().find(|&n| member(svm, n).seat > 0).or(owned.first().copied());
    (member_address(own.unwrap_or(1)), vec![])
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
    let r: sasona::Reading = read(svm, reading);
    let coin = pda(&[COIN_SEED]);
    let accounts = sasona::accounts::UpholdChallenge {
        challenge: key(challenge_address(reading)),
        reading: key(reading),
        members: key(members_address()),
        member: key(member_of(svm, reading)),
        pool: key(pda(&[POOL_SEED])),
        coin_mint: key(coin),
        stakes: key(pda(&[sasona::STAKES_SEED])),
        cover: key(pda(&[COVER_SEED])),
        cover_vault: key(pda(&[COVER_VAULT_SEED])),
        book: key(book_address(r.member)),
        challenger: c.challenger,
        challenger_coin: key(ata(addr(c.challenger), coin)),
        token_program: anchor_spl::token::ID,
    }
    .to_account_metas(None);
    let mut accounts = metas(accounts);
    if member(svm, r.member).seat > 0 {
        accounts.extend(unseat_accounts(svm, r.member));
    }
    Instruction { program_id: program_id(), accounts, data: sasona::instruction::UpholdChallenge {}.data() }
}

/// The seat a membership account sits in, or seat 1's address if it has none
/// or does not exist, so the program is what refuses it.
pub fn seat_of(svm: &LiteSVM, member: Address) -> Address {
    let k = svm
        .get_account(&member)
        .and_then(|a| sasona::Member::try_deserialize(&mut a.data.as_slice()).ok())
        .map(|m| m.seat)
        .unwrap_or(0);
    seat_address(k.max(1))
}

// ------------------------------------------------------------------- quotes

pub fn quote_address(reading: Address) -> Address {
    pda(&[sasona::QUOTE_SEED, reading.as_ref()])
}

pub fn set_quote_ix(svm: &LiteSVM, reader: Address, reading: Address, rate: u16) -> Instruction {
    let accounts = sasona::accounts::SetQuote {
        reader: key(reader),
        reading: key(reading),
        member: key(member_of(svm, reading)),
        quote: key(quote_address(reading)),
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::SetQuote { rate }.data() }
}

// ------------------------------------------------- purchases and chargebacks

pub fn book_address(number: u32) -> Address {
    pda(&[sasona::BOOK_SEED, &number.to_le_bytes()])
}

pub fn book(svm: &LiteSVM, number: u32) -> sasona::Book {
    read(svm, book_address(number))
}

pub fn purchase_address(buyer: Address, id: u64) -> Address {
    pda(&[sasona::PURCHASE_SEED, buyer.as_ref(), &id.to_le_bytes()])
}

pub fn chargeback_address(purchase: Address) -> Address {
    pda(&[sasona::CHARGEBACK_SEED, purchase.as_ref()])
}

pub fn chargeback(svm: &LiteSVM, purchase: Address) -> sasona::Chargeback {
    read(svm, chargeback_address(purchase))
}

pub fn replay_address(chargeback: Address, draw: u8) -> Address {
    pda(&[sasona::READING_SEED, chargeback.as_ref(), &[draw]])
}

/// The merchant's address for tests, and a dollar account it holds.
pub const MERCHANT: Address = Address::new_from_array([77u8; 32]);
pub fn merchant_usd(svm: &mut LiteSVM) -> Address {
    let at = Address::new_from_array([78u8; 32]);
    if svm.get_account(&at).is_none() {
        put_token_account(svm, at, usd(), MERCHANT, 0);
    }
    at
}

/// A dollar account for `who`, made if they have none, at a fixed address.
pub fn usd_of(svm: &mut LiteSVM, who: Address) -> Address {
    let at = ata(who, usd());
    if svm.get_account(&at).is_none() {
        put_token_account(svm, at, usd(), who, 0);
    }
    at
}

/// The merchant's and the quoter's dollar accounts must exist already, as
/// `insured_reading` leaves them.
pub fn buy_ix(svm: &LiteSVM, buyer: Address, buyer_usd: Address, reading: Address, id: u64, price: u64) -> Instruction {
    let rate = svm
        .get_account(&quote_address(reading))
        .and_then(|a| sasona::Quote::try_deserialize(&mut a.data.as_slice()).ok())
        .map(|q| q.rate)
        .unwrap_or(0);
    buy_at_ix(svm, buyer, buyer_usd, reading, id, price, rate)
}

/// A purchase accepting at most `max_rate`.
pub fn buy_at_ix(svm: &LiteSVM, buyer: Address, buyer_usd: Address, reading: Address, id: u64, price: u64, max_rate: u16) -> Instruction {
    let r: sasona::Reading = read(svm, reading);
    let quoter = member(svm, r.member).owner;
    let merchant = Address::new_from_array([78u8; 32]);
    let quoter_usd = ata(addr(quoter), usd());
    let accounts = sasona::accounts::Buy {
        buyer: key(buyer),
        buyer_usd: key(buyer_usd),
        usd_mint: key(usd()),
        pool: key(pda(&[POOL_SEED])),
        reading: key(reading),
        quote: key(quote_address(reading)),
        member: key(member_address(r.member)),
        book: key(book_address(r.member)),
        purchase: key(purchase_address(buyer, id)),
        merchant_usd: key(merchant),
        quoter_usd: key(quoter_usd),
        token_program: anchor_spl::token::ID,
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::Buy { id, price, max_rate }.data() }
}

pub fn close_purchase_ix(svm: &LiteSVM, purchase: Address) -> Instruction {
    let pu: sasona::Purchase = read(svm, purchase);
    let accounts = sasona::accounts::ClosePurchase { purchase: key(purchase), book: key(book_address(pu.member)) }.to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::ClosePurchase {}.data() }
}

pub fn charge_back_ix(svm: &LiteSVM, buyer: Address, buyer_usd: Address, purchase: Address) -> Instruction {
    let pu: sasona::Purchase = read(svm, purchase);
    let accounts = sasona::accounts::ChargeBack {
        buyer: key(buyer),
        buyer_usd: key(buyer_usd),
        usd_mint: key(usd()),
        pool: key(pda(&[POOL_SEED])),
        purchase: key(purchase),
        member: key(member_address(pu.member)),
        members: key(members_address()),
        service_terms: key(pda(&[sasona::SERVICE_SEED, &pu.service])),
        chargeback: key(chargeback_address(purchase)),
        escrow: key(pda(&[sasona::ESCROW_SEED])),
        token_program: anchor_spl::token::ID,
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::ChargeBack {}.data() }
}

pub fn record_draw_ix(purchase: Address) -> Instruction {
    let accounts = sasona::accounts::RecordDraw { chargeback: key(chargeback_address(purchase)), slot_hashes: sasona::SLOT_HASHES_ID }
        .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::RecordDraw {}.data() }
}

/// 7.4, followed against the accounts: the seat drawn for the chargeback's
/// current draw, and the seats passed over before it that must be shown.
/// The draw's entropy must already be recorded.
pub fn replay_drawn_for(svm: &LiteSVM, purchase: Address) -> (Option<u32>, Vec<u32>) {
    let c = chargeback(svm, purchase);
    let mut skipped = vec![];
    if c.draw_members == 0 {
        return (None, skipped);
    }
    let declined = &c.declined[..c.counted_draws as usize];
    let now = seated(svm);
    for a in 0..sasona::MAX_READER_ATTEMPTS {
        let k = sasona::reader_number(&c.seed, &c.service, a, c.draw_members);
        if k > now {
            continue;
        }
        let s = seat(svm, k);
        if s.since >= c.draw_slot || s.owner == c.buyer || s.owner == c.quoter || declined.contains(&s.member) {
            skipped.push(k);
            continue;
        }
        return (Some(k), skipped);
    }
    (None, skipped)
}

/// A replay commitment by `reader`, with the seat drawn and the seats passed
/// over; if they were not drawn, it names their own seat and shows nothing.
pub fn commit_replay_ix(svm: &LiteSVM, reader: Address, purchase: Address, endpoint: &str, question_hash: [u8; 32]) -> Instruction {
    let c = chargeback(svm, purchase);
    let cb = chargeback_address(purchase);
    let (drawn, skipped) = if c.seed == [0u8; 32] { (None, vec![]) } else { replay_drawn_for(svm, purchase) };
    let (number, shown) = match drawn {
        Some(k) if addr(seat(svm, k).owner) == reader => (seat(svm, k).member, skipped),
        _ => {
            let owned: Vec<u32> = (1..=member_count(svm)).filter(|&n| addr(member(svm, n).owner) == reader).collect();
            (owned.iter().copied().find(|&n| member(svm, n).seat > 0).or(owned.first().copied()).unwrap_or(1), vec![])
        }
    };
    let m = member_address(number);
    let accounts = sasona::accounts::CommitReplay {
        reader: key(reader),
        chargeback: key(cb),
        members: key(members_address()),
        member: key(m),
        seat: key(seat_of(svm, m)),
        replay: key(replay_address(cb, c.draw)),
        slot_hashes: sasona::SLOT_HASHES_ID,
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    let mut accounts = metas(accounts);
    accounts.extend(shown.into_iter().map(|k| AccountMeta::new_readonly(seat_address(k), false)));
    let data = sasona::instruction::CommitReplay { endpoint: endpoint.to_string(), question_hash }.data();
    Instruction { program_id: program_id(), accounts, data }
}

/// Pass the current draw, naming the seat it drew with its proof, if any.
pub fn pass_draw_ix(svm: &LiteSVM, purchase: Address) -> Instruction {
    let c = chargeback(svm, purchase);
    let cb = chargeback_address(purchase);
    let (drawn, skipped) = if c.seed == [0u8; 32] { (None, vec![]) } else { replay_drawn_for(svm, purchase) };
    let accounts = sasona::accounts::PassDraw {
        chargeback: key(cb),
        members: key(members_address()),
        replay: key(replay_address(cb, c.draw)),
        slot_hashes: sasona::SLOT_HASHES_ID,
    }
    .to_account_metas(None);
    let mut accounts = metas(accounts);
    if let Some(k) = drawn {
        accounts.push(AccountMeta::new_readonly(seat_address(k), false));
    }
    accounts.extend(skipped.into_iter().map(|k| AccountMeta::new_readonly(seat_address(k), false)));
    let data = sasona::instruction::PassDraw { drawn_seat: drawn.unwrap_or(0) }.data();
    Instruction { program_id: program_id(), accounts, data }
}

pub fn settle_chargeback_ix(svm: &mut LiteSVM, purchase: Address) -> Instruction {
    let c = chargeback(svm, purchase);
    let cb = chargeback_address(purchase);
    let replay = replay_address(cb, c.draw);
    let replayer = svm
        .get_account(&replay)
        .and_then(|a| sasona::Reading::try_deserialize(&mut a.data.as_slice()).ok())
        .filter(|r| r.state == sasona::READING_REVEALED)
        .map(|r| addr(r.reader));
    let replayer_usd = replayer.map(|r| usd_of(svm, r));
    let buyer_usd = usd_of(svm, addr(c.buyer));
    let pu: sasona::Purchase = read(svm, purchase);
    let accounts = sasona::accounts::SettleChargeback {
        chargeback: key(cb),
        purchase: key(purchase),
        book: key(book_address(pu.member)),
        replay: key(replay),
        pool: key(pda(&[POOL_SEED])),
        coin_mint: key(pda(&[COIN_SEED])),
        pool_usd: key(pda(&[POOL_USD_SEED])),
        pool_coin: key(pda(&[POOL_COIN_SEED])),
        fees: key(pda(&[FEES_SEED])),
        cover: key(pda(&[COVER_SEED])),
        cover_vault: key(pda(&[COVER_VAULT_SEED])),
        escrow: key(pda(&[sasona::ESCROW_SEED])),
        buyer_usd: key(buyer_usd),
        replayer_usd: replayer_usd.map(key),
        token_program: anchor_spl::token::ID,
    }
    .to_account_metas(None);
    Instruction { program_id: program_id(), accounts: metas(accounts), data: sasona::instruction::SettleChargeback {}.data() }
}

pub fn repay_cover_ix(svm: &LiteSVM, purchase: Address) -> Instruction {
    let c = chargeback(svm, purchase);
    let accounts = sasona::accounts::RepayCover {
        chargeback: key(chargeback_address(purchase)),
        book: key(book_address(c.member)),
        member: key(member_address(c.member)),
        members: key(members_address()),
        pool: key(pda(&[POOL_SEED])),
        coin_mint: key(pda(&[COIN_SEED])),
        stakes: key(pda(&[sasona::STAKES_SEED])),
        cover: key(pda(&[COVER_SEED])),
        cover_vault: key(pda(&[COVER_VAULT_SEED])),
        token_program: anchor_spl::token::ID,
    }
    .to_account_metas(None);
    let mut accounts = metas(accounts);
    // If what is taken leaves less than a whole stake, the membership leaves its seat.
    let m = member(svm, c.member);
    if m.seat > 0 && m.stake.saturating_sub(c.owed_coins) < sasona::MEMBER_STAKE {
        accounts.extend(unseat_accounts(svm, c.member));
    }
    Instruction { program_id: program_id(), accounts, data: sasona::instruction::RepayCover {}.data() }
}

/// In a world with the pool open: membership 1 (the depositor) reads
/// `service` in a round drawn at `slot`, finds it delivering, records the
/// merchant's address, and quotes it at `rate`. Returns the reading.
pub fn insured_reading(w: &mut World, service: &str, rate: u16, slot: u64) -> Address {
    if member_count(&w.svm) == 0 {
        let d = w.depositor.insecure_clone();
        join(&mut w.svm, &d);
    }
    let d = w.depositor.insecure_clone();
    let fp = sha256(service.as_bytes());
    let seed = sha256(&fp);
    w.svm.warp_to_slot(slot);
    try_ix(&mut w.svm, open_round_ix(d.pubkey(), fp, 10, 2, sha256(&seed)), &d).unwrap();
    at_slot(&mut w.svm, slot + 40, &recent(slot + 40, &[]));
    let round = round_address(fp);
    try_ix(&mut w.svm, reveal_ix(round, d.pubkey(), seed), &d).unwrap();
    let nonce: [u8; 16] = fp[..16].try_into().unwrap();
    let q = sha256(&sasona::canonical_question(&nonce));
    try_with(&mut w.svm, |s| commit_reading_ix(s, d.pubkey(), round, service, q), &d).unwrap();
    let reading = reading_address(round, service);
    w.svm.warp_to_slot(slot + 41);
    try_ix(&mut w.svm, reveal_paid_ix(d.pubkey(), reading, nonce, [9u8; 32], 1, MERCHANT), &d).unwrap();
    merchant_usd(&mut w.svm);
    usd_of(&mut w.svm, d.pubkey());
    try_with(&mut w.svm, |s| set_quote_ix(s, d.pubkey(), reading, rate), &d).unwrap();
    reading
}

/// Someone new with `dollars` to spend, in an account `usd_of` finds.
pub fn new_buyer(svm: &mut LiteSVM, dollars: u64) -> Keypair {
    let k = Keypair::new();
    svm.airdrop(&k.pubkey(), 10_000_000_000).unwrap();
    put_token_account(svm, ata(k.pubkey(), usd()), usd(), k.pubkey(), dollars);
    k
}

/// Pay `dollars` back out of the cover the way it now happens: a covered
/// purchase, a chargeback, and 7 days with no replay. Returns the buyer's
/// dollar account, which ends holding exactly `dollars`.
pub fn pay_back(w: &mut World, dollars: u64) -> Result<Address, String> {
    const SERVICE: &str = "https://paid-back.example/run";
    let reading = reading_address(round_address(sha256(SERVICE.as_bytes())), SERVICE);
    if w.svm.get_account(&reading).is_none() {
        let slot = w.svm.get_sysvar::<solana_clock::Clock>().slot.max(1) + 1_000;
        insured_reading(w, SERVICE, 100, slot);
    }
    let buyer = new_buyer(&mut w.svm, dollars + dollars / 10 + 1);
    let usd_acc = ata(buyer.pubkey(), usd());
    let id = 1;
    try_with(&mut w.svm, |s| buy_ix(s, buyer.pubkey(), usd_acc, reading, id, dollars), &buyer)?;
    let purchase = purchase_address(buyer.pubkey(), id);
    try_with(&mut w.svm, |s| charge_back_ix(s, buyer.pubkey(), usd_acc, purchase), &buyer)?;
    // Spend what is left, so the account holds only what comes back.
    put_token_account(&mut w.svm, usd_acc, usd(), buyer.pubkey(), 0);
    days_pass(&mut w.svm, 8);
    let ix = settle_chargeback_ix(&mut w.svm, purchase);
    try_ix(&mut w.svm, ix, &buyer)?;
    Ok(usd_acc)
}

/// Coin held by `who`.
pub fn coins(svm: &LiteSVM, who: Address) -> u64 {
    token_balance(svm, ata(who, pda(&[sasona::COIN_SEED])))
}

/// Someone with coin to put up a challenge bond from.
pub fn challenger(svm: &mut LiteSVM) -> Keypair {
    let (k, usd) = newcomer(svm, 10);
    try_deposit(svm, &k, usd, 10 * DOLLAR).unwrap();
    k
}
