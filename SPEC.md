# Shielded Tab v0 protocol

Status: research prototype; Zcash testnet and FLOP devnet only. This document is normative for fields and bytes.

## Quote and commitment

A canonical session identifier is `flop-devnet:<genesis-hash-lowercase-hex>:<owner-ss58>:<delegate-ss58>`. The genesis hash is 32 bytes without `0x`; SS58 accounts must decode and re-encode byte-for-byte. v0 requires FLOP genesis `99c172c095340ace980631f9123544c6bebd4839f3437e463533422d8be2d631`.

Let `C = BLAKE2b-256(personal="ShieldedTab_C_v0", preimage)`. Personalization is exactly the 16 ASCII bytes shown. The preimage is seven fields, each encoded `u32_be(byte_length) || bytes`, in this exact order:

1. version: one byte `00`;
2. Zcash network: UTF-8 `test`;
3. receiving address: exact canonical testnet shielded UA or Sapling address in the payment request;
4. amount: unsigned 64-bit big-endian zatoshis;
5. session id: canonical UTF-8 string above;
6. nonce: 32 random bytes;
7. quote expiry: unsigned 32-bit big-endian Zcash height.

The version fixes interpretation. Network prevents cross-network reuse. Address and amount prevent redirect and amount substitution. Session binds the receipt to the exact owner/delegate pair. Nonce prevents quote collisions. Expiry limits stale quotes and replay. The quote id is a public 16-byte random identifier and is memo metadata; it is not part of C.

### Test vector 1

All values below are public test values, not funded accounts. Address `ztestsapling1vector0nly`, amount `150000`, session `flop-devnet:99c172c095340ace980631f9123544c6bebd4839f3437e463533422d8be2d631:5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY:5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty`, nonce `11` repeated 32 times, expiry `4200000` yields:

`C = 2cd62794f5b5bb726e0d2bfaebfb9b8a4f0ac007518e1a1693fff55efbd85bd3`

## Memo

v0 uses ZIP-302 text memo bytes, ASCII `STAB0:<C lowercase hex>:<nonce lowercase hex>:<quote_id lowercase hex>:<amount_zat decimal>:<expiry_height decimal>`, then NUL padding to 512 bytes. Decimal fields have no leading zero except `0`. The memo starts with ASCII `S` (below `0xF5`); bytes `0xF5` and `0xFF` are not used as first-byte special markers. A STAB-family prefix with invalid version, lengths, case, separators, decimal form, or nonzero trailing bytes is malformed and must fail closed. Other memos are unrelated. The nonce, amount, and expiry are visible only to recipients/viewing-key holders and let them recompute C without broker quote state.

For vector 1 with quote id `22` repeated 16 times, the full unpadded memo text is:

`STAB0:2cd62794f5b5bb726e0d2bfaebfb9b8a4f0ac007518e1a1693fff55efbd85bd3:1111111111111111111111111111111111111111111111111111111111111111:22222222222222222222222222222222:150000:4200000`

The 512-byte memo is these 183 ASCII bytes followed by 329 NUL bytes.

## FLOP record

After a matched payment, the broker submits `utility.batchAll([flopSessionKeys.createSession(delegate, spendingCap, spendingCap, spendingCap, durationBlocks, []), system.remarkWithEvent(UTF8("STAB0:") || C)])` from the broker/owner key. The metadata call takes delegate, spending cap, per-tx cap, daily cap, duration blocks, and allowed pallets. The remark bytes are exactly 70 ASCII bytes: `STAB0:` followed by 64 lowercase hexadecimal characters encoding C. No Zcash txid, amount, address, nonce, or quote id is written on FLOP. The adapter checks devnet genesis before building or submitting and requires `ExtrinsicSuccess` and `System.Remarked`. Atomic batch failure must leave neither call applied.

For `System.Remarked`, metadata defines `hash` as `blake2_256(remark_bytes)`, not the remark bytes themselves. The adapter therefore checks the event's sender against the owner and its hash against BLAKE2-256 of the exact 70 remark bytes. It does not search event data for `STAB0:` or C directly.

## Verification

Given a testnet viewing key, mined txid, session id, optional receiving address, and (where needed) FLOP block hint: fetch and parse the exact transaction or use an offline raw transaction file; decrypt incoming Sapling/Orchard outputs using official APIs; locate a valid STAB0 memo; take amount and expiry from that memo; recompute C using the supplied address and session id; require decrypted value at least the memo amount and mined height no greater than expiry; query the FLOP session for the owner/delegate pair; and find the broker-originated `STAB0:` remark in the specified block. Address defaults to the viewing key's default unified address. No broker quote database is required. Report success only if all checks agree.

The exact remark byte vector for vector 1 is UTF-8 `STAB0:2cd62794f5b5bb726e0d2bfaebfb9b8a4f0ac007518e1a1693fff55efbd85bd3` (70 bytes, hex `53544142303a32636436323739346635623562623732366530643262666165626662396238613466306163303037353138653161313639336666663535656662643835626433`).

Malformed data, mainnet network markers, changed fields, unmined txs, duplicate quote/tx use, expiry, absent session/remark, endpoint chain mismatch, or ambiguous results are failures. New field layouts or semantics require a new magic/version and personalization. v0 has no Zcash signing or spending path.
