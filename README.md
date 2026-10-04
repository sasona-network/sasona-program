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
| `request_release` | Asks for some of your guarantee back. It stays in the cover, still paying claims, for 45 days. |
| `release` | After the notice, pays your shares' part of the cover to you as coins. |
| `join_cover` | A one-off: moves a guarantee made before the cover existed into it. |
| `open_round` | Commits a draw: the list of services by its fingerprint, how many to pick, and the hash of a secret seed, with a 0.1 SOL bond. A list can be drawn once. |
| `reveal_round` | Reveals the seed and mixes it with the hash of a Solana slot that did not exist at commit time. The bond goes back. The rule that turns the result into picks is in [sasona-protocol](https://github.com/sasona-network/sasona-protocol). |
| `mark_withheld` | Marks a round whose seed was not revealed in time. It can never be drawn, and the bond is lost. |
| `commit_reading` | Before a drawn service is called, records the hash of the question for it. |
| `reveal_reading` | After the call, reveals the nonce, the reply's hash, the verdict and the address the service asked to be paid at. The program builds the question from the nonce and refuses unless it is the one committed. A nonce belongs to the reading that committed to it first. |
| `mark_lapsed` | Marks a reading not revealed within about an hour. |
| `commit_second_reading` | A reading in a re-read round that names the earlier reading it tests again: the same service, someone else's reading, revealed before the round was committed. |
| `settle_pair` | Once the second reading is revealed, records what the two settle: works now, false or decayed, or agreed fails. Anyone can call it. |
| `join_members` | Locks one stake of coin as a membership, which sits in the seat after the last. |
| `ask_to_leave` | Takes a membership off the roster at once; the membership in the last seat moves into its seat. |
| `leave` | After 45 days' notice, and with no challenge open, gives the stake back. |
| `challenge` | Challenges a reading within 30 days of its reveal, for a 0.1 SOL bond. |
| `open_evidence`, `write_evidence` | The reader puts a reading's reply on chain, in pieces. |
| `answer_challenge` | Holds if the reply on chain hashes to what was recorded and gives the recorded verdict for the nonce. The bond goes to the member and the reply is sealed. |
| `uphold_challenge` | After 7 days with no answer that held: the reading stops counting and the membership loses its stake. What it owes the cover comes out first, a tenth of the rest goes to the challenger, and the rest to the cover. |
| `set_quote` | The member who took a reading sets, changes or withdraws what they would charge to insure a purchase from the service, in basis points. Only on a reading that says delivered, within 30 days, while the member is active. |
| `buy` | A purchase covered by a quote. The price goes to the address the reading recorded, the premium to the member who quoted. Refused past the member's room to insure, or if the quote was raised past the rate the buyer accepts. |
| `close_purchase` | After 7 days with no chargeback, gives the member back the room the purchase took. Anyone can call it. |
| `charge_back` | Within 7 days, the buyer asks for the price back, with a 5% deposit unless nobody charged back the service in the last 30 days. Draws a member to replay the service. |
| `record_draw` | Records the entropy of a replay's draw, which fixes the member drawn. |
| `commit_replay` | The drawn member commits a reading of the service, as for a round. It is revealed with `reveal_reading`. |
| `pass_draw` | After the hour to read, counts a draw nobody used, and draws again. After 8 the chargeback can be settled. |
| `settle_chargeback` | Pays out what the replay decided: the buyer back out of the cover, or the deposit to the replayer. 7 days with no replay pays the buyer. |
| `repay_cover` | Once the covering reading can no longer be challenged, takes what the member owes the cover out of their stake. A membership left with less than a whole stake leaves its seat. |

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
- Each reading is taken by the member drawn for it, but the round's opener still writes the list and picks when to open it.
- The stake, the challenge bond, the challenger's tenth, the minimum covered price and the 5% are devnet figures. A stake that deters has to grow with the traffic a service carries, now that purchases are on chain.
- Once its notice has run out, a guarantee can be released just ahead of a claim its owner can see coming. Releases do not pause yet while a chargeback is open.

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
