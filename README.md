# Sasona program

The Solana contract that holds the network's money.

It holds what buyers pay and what members stake, and it pays out only under its own rules. Coins are only created when someone deposits dollars, and the only thing that can create them is the pool's own address, which has no private key.

> Devnet only. The coin has no value.

## On devnet

| | |
|---|---|
| Program | `7eiHSnDkM4WjJdY36D2Yqsjw893mCMUtBAwMCQ5adL99` |
| Dollar (a stand-in we created) | `ACPdQtaC6HKgT8V4GVRke57vtZrbv3zDqTwvENBoRPCy` |

While on devnet the program can still be upgraded by its deployer. Each proof in the [roadmap](https://github.com/sasona-network/roadmap) names that key.

## What it does so far

| Instruction | What it does |
|---|---|
| `open` | Creates the coin and the pool, and makes the first deposit. The dollar and the opening price are fixed in the program, so whoever opens it gains nothing by going first. |
| `deposit` | Deposits into the open pool at the pool's own price. The pool mints its side to match, so the price does not move. A repeat deposit adds to the same guarantee. |
| `pay_fee` | Pays the markup on a purchase, in dollars. Five of its fifteen points stay in the pool as depth, 3% of it buys coin that is burned, and the rest buys coin for the participants. |
| `settle_entry_fees` | Turns the entry fees deposits left waiting into coin the same way, without the reserve. Anyone can call it. |
| `add_depth` | Adds dollars to the pool with nothing minted against them, which raises the price. Anyone can call it. |
| `claim` | Pays a buyer back in dollars out of the cover. Equal coins are burned from the pool, so the price does not fall, and from the cover, so every guarantee carries the loss alike. |
| `request_release` | Asks for some of your guarantee back. It stays in the cover, still paying claims, for 45 days. |
| `release` | After the notice, pays your shares' part of the cover to you as coins. |
| `join_cover` | A one-off: moves a guarantee made before the cover existed into it. |
| `open_round` | Commits a draw: the list of services by its fingerprint, how many to pick, and the hash of a secret seed, with a 0.1 SOL bond. A list can be drawn once. |
| `reveal_round` | Reveals the seed and mixes it with the hash of a Solana slot that did not exist at commit time. The bond goes back. The rule that turns the result into picks is in [sasona-protocol](https://github.com/sasona-network/sasona-protocol). |
| `mark_withheld` | Marks a round whose seed was not revealed in time. It can never be drawn, and the bond is lost. |

A deposit is split four ways:

| Part | Share | Where it goes |
|---|---|---|
| Entry fee | 15% of the deposit | Held until settled into coin: 3% burned, the rest to the participants |
| Spread | 15% of the rest | Stays in the pool as depth |
| Guarantee | 75% of the rest | Shares of the cover that pays buyers back, held by the pool |
| Free | 10% of the rest | The depositor's own coins |

## Temporary, and will change

- None of the participant roles exists on chain yet, so their share of every fee goes to the network, and the network's coin waits in a vault held by the pool with no way out. Both change when developers, members, submitters and marketers come on chain.
- Any caller can pay any fee, because there is no purchase on chain to tie it to yet. It has to be tied to real purchases before participants are paid.
- Claims are approved by the key that can already upgrade the program. Members drawn at random replace it in part 7 of the roadmap.
- Once its notice has run out, a guarantee can be released just ahead of a claim its owner can see coming. Releases will pause while a claim is pending, once claims are filed on chain (part 7).

## Build and test

You need Linux or WSL, Rust, the Solana CLI and Anchor.

```bash
bash scripts/build.sh
```

This builds the program and runs the tests against the compiled binary in a local simulator. Several tests are attacks, and they pass when the attack is refused.

```bash
bash scripts/mutate.sh
```

This breaks the program in specific ways, one at a time, and checks that the tests catch each one.

## Known limit

At 5,000 coins a dollar the coin's supply counter fills after about $2 billion of total deposits. The price or the coin's decimals will be set before mainnet so that limit is out of reach.
