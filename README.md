# Shielded Tab

Research prototype for a broker that accepts Zcash TESTNET payments and opens a FLOP DEVNET agent session. A ZIP-321 request carries an encrypted STAB0 memo with a commitment to the quote and the FLOP owner/delegate session. The broker scans with a viewing key and writes only `STAB0:` plus the commitment to FLOP. There is no bridge and no Zcash spending path.

The protocol bytes and vector are in [SPEC.md](SPEC.md). Rust pins the official librustzcash crates and produces `stab-zec`. Node 24 ESM uses exactly `@flop-labs/sdk` 0.7.0 and `@flop-labs/cli` 0.6.0.

## Offline quickstart

```sh
cargo test --workspace --locked
(cd broker && npm ci && npm test)
./scripts/e2e-offline.sh
```

For a broker testnet wallet, create a private directory outside this repo (`chmod 700 ~/stab-secrets`), then run `cargo run -p stab-zec -- keygen-testnet --seed-out ~/stab-secrets/broker.seed`. The command refuses an existing file and writes mode 0600; store only the printed UFVK in a separate protected file for `STAB_UFVK_FILE`.

A ZIP-321 request can be produced with `stab-broker quote --address <testnet-shielded-address> --owner <owner-ss58> --delegate <delegate-ss58> --amount <zatoshis> --expiry <testnet-height>`. Then scan with `stab-zec scan --ufvk-file ... --start <birthday>`. `stab-zec chain-info` checks the lightwalletd-reported chain before scanning. Default endpoint is `https://testnet.zec.rocks:443`.

## Live testnet workflow

Build with Rust 1.85.1 and run `npm ci` under Node 24. Keep UFVK and FLOP keystore/passphrase in protected files outside the repo. Run `chain-info`, verify `chain_name` is `test`, then scan a bounded range before operating a watcher. FLOP devnet requires `FLOP_KEYSTORE`, `FLOP_KEYSTORE_PASS_FILE`, and `FLOP_RPC` (default `wss://rpc.devnet.flop.network`). The adapter verifies the expected genesis and batches `createSession` with `remarkWithEvent`. Use `--dry-run` to inspect the call before submitting. Never use `flop proxy`.

## Architecture and flow

```text
agent ── session id ──> broker quote ── ZIP-321 URI ──> Zcash TESTNET
                            │                              │
                            │ C, quote state               │ encrypted STAB0 memo
                            ▼                              ▼
                      broker watcher <── lightwalletd compact blocks + tx fetch
                            │
                            ├── FLOP DEVNET batch: createSession + remarkWithEvent(STAB0:||C)
                            └── proof inputs: viewing key + txid + session id (+ address)
```

1. `quote` binds the canonical FLOP owner/delegate pair, testnet receiving address, amount, random nonce, and expiry into C. It emits a ZIP-321 request whose memo encrypts C and the public quote fields to the receiving wallet.
2. `scan` trial-decrypts Sapling and Orchard compact outputs, fetches full transactions for hits, and emits JSONL with decrypted value and memo.
3. `watch` matches the mined height and memo, then atomically creates the session and records only `STAB0:` plus C on FLOP.
4. `verify` decrypts the named transaction and recomputes C from the request address and session id. The memo supplies amount and expiry, so this check needs no broker quote database. FLOP verification separately checks the owner, session, and remark.

## CLI reference

| Command | Inputs and result |
| --- | --- |
| `stab-zec request` | `--address --amount-zat --session-id --nonce-hex --expiry-height --quote-id-hex`; prints ZIP-321 URI and C. |
| `stab-zec commit` | Same quote fields; prints C. |
| `stab-zec parse-request` | `--uri`; parses the URI and memo. |
| `stab-zec chain-info` | Optional `--endpoint` or offline `--fixture-dir`; accepts only the Zcash test chain. |
| `stab-zec scan` | `--ufvk-file --start [--end] [--endpoint]`; emits one decrypted output per JSON line. `STAB_UFVK_FILE` may supply the key path. |
| `stab-zec decrypt-tx` | `--ufvk-file` plus either `--txid` (online) or `--tx-hex-file --height` (offline). |
| `stab-zec ufvk-address` | `--ufvk-file`; prints its default testnet unified address. |
| `stab-zec keygen-testnet` | `--seed-out`; creates a new 0600 test seed and prints its UFVK/address. |
| `stab-broker quote` | `--address --owner [--delegate] --amount --expiry [--spending-cap --duration-blocks]`; generates delegate credentials if omitted and writes quote state under ignored `broker/state/`. |
| `stab-broker watch` | `--input JSONL` or `STAB_UFVK_FILE --start`; supports `--adapter mock|devnet`, `--tip`/`--end`, and devnet `--dry-run`. |
| `stab-broker verify` | `--viewing-key --txid --session-id [--address] [--tx-hex-file --height] [--flop-block] [--adapter mock|devnet]`; emits a JSON proof result. |

## Status

Offline coverage builds a v5 testnet transaction at test time from the explicitly TEST ONLY seed `[7; 32]`. It has one encrypted Sapling output and one encrypted Orchard output to the derived testnet UA. Tests decrypt both exact 100,000-zatoshi values and STAB0 memos through the library and CLI, compact-scan both pools, reject another UFVK and refuse a mainnet UFVK. The builder uses mock proofs: the fixture is for decryption and scanning only and is not a transaction to broadcast.

The offline end-to-end script runs quote -> encrypted fixture -> `decrypt-tx` -> mock FLOP watch -> verify. It checks wrong session, wrong address, missing FLOP record, and replay failures. Current recorded suite counts are 32 Rust tests (30 unit + 2 encrypted-fixture integration), 30 broker tests, and the same suites green inside `scripts/e2e-offline.sh`.

Live checks (2026-11-19):

- `stab-zec chain-info` against https://testnet.zec.rocks:443 reported chain `test`, height 4,639,858, ECC LightWalletD v0.6.1.
- `stab-zec scan` scanned testnet blocks 4,639,800 to 4,639,858 with a freshly generated broker testnet UFVK: 59 blocks, 0 hits (expected; the wallet is unfunded).
- On FLOP devnet (genesis 0x99c172c0…2d631, runtime 1014029), the broker opened a real session with `utility.batchAll(createSession, remarkWithEvent)` from a separate devnet-only key at block 4,126,237 (hash 0xe24649accc96024972ee203b0deaa3174954c95279803f73706663f6368aefbd), with commitment c24a0f7a1736134dead8283cfa2bd07096398137308693337f41b38a1063171a, funded by the offline encrypted fixture payment. `verify --adapter devnet --flop-block 4126237` returned ok; the same verify against block 4,126,236 failed as expected.
- Not yet done live: a real shielded testnet ZEC payment (needs testnet ZEC from a faucet), so the Zcash leg of the live run used the offline fixture transaction.

No mainnet operation is supported.

## Threat model and disclosure

The broker learns incoming values, transaction ids, timing, memo contents, the session identities, and quote metadata. It is trusted to match correctly and protect the viewing key. A FLOP observer learns that an owner/delegate session was created and sees C and its timing; C does not reveal amount/address/txid without opening the quote or memo. Amount and timing correlations can still link activity. Public remarks can be copied or spoofed by other accounts; verification must establish the broker account as remark origin. Replay is limited by quote expiry, unique nonce/id, and broker state, but a compromised or inconsistent broker database can still cause denial or duplicate work. Front-running and quote disclosure remain operational risks.

A Sapling incoming viewing key (IVK) reveals incoming Sapling notes but not outgoing notes. A UFVK is broader; disclosure scope depends on its pool components. Give auditors an incoming-only view where possible, rather than a full viewing key. Memo contents are linkable to anyone who can decrypt them. A lightwalletd server learns queried ranges and network metadata, and learns the txid through `GetTransaction` for each trial-decryption hit. This scanner does not provide privacy-preserving retrieval and trusts the server for completeness. Broker key compromise exposes incoming payment activity and can permit unauthorized FLOP session creation under the broker account; isolate keys and limit runtime access. Verify results independently if completeness matters.

On 2026-05-29, Taylor Hornby reported an Orchard Action circuit soundness bug that could have allowed balance violation and theft of funds. Orchard was disabled at height 3,363,426 and re-enabled in NU6.2 at height 3,364,600 with the fixed circuit specified by ZIP 257. This was a circuit soundness issue, not a decryption bug. This prototype pins crate versions and has not been audited against every network upgrade.

This is research software, testnet only, and not financial software. A broker is the on-ramp and may have KYC/MSB obligations depending on jurisdiction. Article 79 of the [EU AML Regulation (AMLR), Regulation (EU) 2024/1624](https://eur-lex.europa.eu/eli/reg/2024/1624/oj/eng/pdf) prohibits covered entities from maintaining accounts that anonymise or increase obfuscation of transactions, including through anonymity-enhancing coins; the Regulation generally applies from 10 July 2027. Obtain qualified legal advice for deployment. The design favours selective disclosability via viewing keys, but disclosure does not by itself establish legal compliance.
