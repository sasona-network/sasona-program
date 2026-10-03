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

A deposit is split four ways:

| Part | Share | Where it goes |
|---|---|---|
| Entry fee | 15% of the deposit | Held for the fee split, which comes in a later step |
| Spread | 15% of the rest | Stays in the pool as depth |
| Guarantee | 75% of the rest | Locked in a vault the depositor cannot move |
| Free | 10% of the rest | The depositor's own coins |

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
