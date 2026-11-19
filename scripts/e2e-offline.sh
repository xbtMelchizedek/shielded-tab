#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test --workspace --locked
(cd broker && npm test)
tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir" broker/state' EXIT
rm -rf broker/state
mkdir -m 700 broker/state
cargo build -p stab-zec --bin stab-zec --locked
export STAB_ZEC="$PWD/target/debug/stab-zec"
cargo run -q -p stab-zec --example fixture --locked -- "$tmp_dir/default-fixture.json"
node --input-type=module - "$tmp_dir/default-fixture.json" "$tmp_dir/ufvk" <<'JS'
import { readFile, writeFile } from 'node:fs/promises';
const [fixturePath, keyPath] = process.argv.slice(2);
const fixture = JSON.parse(await readFile(fixturePath, 'utf8'));
await writeFile(keyPath, fixture.ufvk + '\n', { mode: 0o600 });
console.log(fixture.address);
JS
address=$(node --input-type=module - "$tmp_dir/default-fixture.json" <<'JS'
import { readFile } from 'node:fs/promises';
console.log(JSON.parse(await readFile(process.argv[2], 'utf8')).address);
JS
)
owner='5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY'
delegate='5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty'
node broker/src/cli.mjs quote --address "$address" --amount 100000 --expiry 4200000 --owner "$owner" --delegate "$delegate" > "$tmp_dir/quote.json"
node --input-type=module - "$tmp_dir/quote.json" "$tmp_dir/quote-memo.json" <<'JS'
import { readFile, writeFile } from 'node:fs/promises';
const [quotePath, memoPath] = process.argv.slice(2);
const { quote } = JSON.parse(await readFile(quotePath, 'utf8'));
await writeFile(memoPath, JSON.stringify(quote));
JS
cargo run -q -p stab-zec --example fixture --locked -- "$tmp_dir/fixture.json" "$tmp_dir/quote-memo.json"
node --input-type=module - "$tmp_dir/fixture.json" "$tmp_dir/ufvk" "$tmp_dir/tx.hex" <<'JS'
import { readFile, writeFile } from 'node:fs/promises';
const [fixturePath, keyPath, txPath] = process.argv.slice(2);
const fixture = JSON.parse(await readFile(fixturePath, 'utf8'));
await writeFile(keyPath, fixture.ufvk + '\n', { mode: 0o600 });
await writeFile(txPath, fixture.tx_hex + '\n', { mode: 0o600 });
JS
txid=$(node --input-type=module - "$tmp_dir/fixture.json" <<'JS'
import { readFile } from 'node:fs/promises';
console.log(JSON.parse(await readFile(process.argv[2], 'utf8')).txid);
JS
)
height=$(node --input-type=module - "$tmp_dir/fixture.json" <<'JS'
import { readFile } from 'node:fs/promises';
console.log(JSON.parse(await readFile(process.argv[2], 'utf8')).height);
JS
)
target/debug/stab-zec decrypt-tx --ufvk-file "$tmp_dir/ufvk" --tx-hex-file "$tmp_dir/tx.hex" --height "$height" > "$tmp_dir/decrypted.json"
node --input-type=module - "$tmp_dir/decrypted.json" "$tmp_dir/outputs.jsonl" <<'JS'
import { readFile, writeFile } from 'node:fs/promises';
const [input, output] = process.argv.slice(2);
const { outputs } = JSON.parse(await readFile(input, 'utf8'));
await writeFile(output, outputs.map(x => JSON.stringify(x)).join('\n') + '\n');
JS
node broker/src/cli.mjs watch --input "$tmp_dir/outputs.jsonl" --tip 4200010 --adapter mock > "$tmp_dir/watch.json"
node --input-type=module - "$tmp_dir/quote.json" <<'JS' > "$tmp_dir/session-id"
import { readFile } from 'node:fs/promises';
console.log(JSON.parse(await readFile(process.argv[2], 'utf8')).quote.session_id);
JS
session=$(cat "$tmp_dir/session-id")
node broker/src/cli.mjs verify --viewing-key "$tmp_dir/ufvk" --txid "$txid" --session-id "$session" --address "$address" --tx-hex-file "$tmp_dir/tx.hex" --height "$height" > "$tmp_dir/verify.json"
node --input-type=module - "$tmp_dir/verify.json" "$tmp_dir/watch.json" <<'JS'
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
assert.equal(JSON.parse(await readFile(process.argv[2], 'utf8')).ok, true);
assert.equal(JSON.parse(await readFile(process.argv[3], 'utf8')).used.length, 2);
JS
wrong_session=$(node --input-type=module - "$session" <<'JS'
const s = process.argv[2];
console.log(s.slice(0, -1) + (s.endsWith('1') ? '2' : '1'));
JS
)
if node broker/src/cli.mjs verify --viewing-key "$tmp_dir/ufvk" --txid "$txid" --session-id "$wrong_session" --address "$address" --tx-hex-file "$tmp_dir/tx.hex" --height "$height"; then exit 1; fi
if node broker/src/cli.mjs verify --viewing-key "$tmp_dir/ufvk" --txid "$txid" --session-id "$session" --address 'utest1wrongfixtureaddress' --tx-hex-file "$tmp_dir/tx.hex" --height "$height"; then exit 1; fi
replay=$(node broker/src/cli.mjs watch --input "$tmp_dir/outputs.jsonl" --tip 4200010 --adapter mock)
REPLAY="$replay" node --input-type=module -e 'import assert from "node:assert/strict"; assert.deepEqual(JSON.parse(process.env.REPLAY).used, [])'
rm broker/state/flop-mock.json
if node broker/src/cli.mjs verify --viewing-key "$tmp_dir/ufvk" --txid "$txid" --session-id "$session" --address "$address" --tx-hex-file "$tmp_dir/tx.hex" --height "$height"; then exit 1; fi
