#!/usr/bin/env node
import { randomBytes } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { commitment, matchPayment, refuseMainnet } from './core.mjs';
import { MockFlopAdapter } from './mock.mjs';
import { devnetAdapter } from './devnet.mjs';
import { Keyring } from '@polkadot/keyring';
import { mnemonicGenerate } from '@polkadot/util-crypto';

const [cmd, ...args] = process.argv.slice(2);
const opts = {};
for (let i = 0; i < args.length; i++) {
  if (args[i].startsWith('--')) opts[args[i].slice(2)] = args[i + 1]?.startsWith('--') ? true : args[++i];
}
const stateDir = new URL('../state/', import.meta.url);
const stateFile = new URL('../state/quotes.json', import.meta.url);
const adapterFile = new URL('../state/flop-mock.json', import.meta.url);
async function load() { try { return JSON.parse(await readFile(stateFile, 'utf8')); } catch { return []; } }
async function save(qs) { await mkdir(stateDir, { recursive: true, mode: 0o700 }); await writeFile(stateFile, JSON.stringify(qs, null, 2), { mode: 0o600 }); }
function runZec(argv) { const p = spawnSync(process.env.STAB_ZEC ?? '../target/debug/stab-zec', argv, { encoding: 'utf8' }); if (p.status !== 0) throw new Error(p.stdout || p.stderr); return JSON.parse(p.stdout); }

if (cmd === 'quote') {
  const address = refuseMainnet(opts.address);
  const owner = opts.owner;
  if (!owner) throw new Error('--owner required');
  let delegate = opts.delegate;
  if (!delegate) {
    const phrase = mnemonicGenerate();
    const pair = new Keyring({ type: 'sr25519' }).addFromUri(phrase);
    delegate = pair.address;
    await mkdir(stateDir, { recursive: true, mode: 0o700 });
    await writeFile(new URL('../state/delegate.seed', import.meta.url), `${phrase}\n`, { mode: 0o600, flag: 'wx' });
  }
  const nonce = randomBytes(32).toString('hex'), quote_id = randomBytes(16).toString('hex');
  const q = { network: 'test', address, amount_zat: String(opts.amount), owner, delegate, session_id: `flop-devnet:${'99c172c095340ace980631f9123544c6bebd4839f3437e463533422d8be2d631'}:${owner}:${delegate}`, nonce, quote_id, expiry_height: Number(opts.expiry), spending_cap: String(opts['spending-cap'] ?? '1'), duration_blocks: Number(opts['duration-blocks'] ?? 1000), label: 'Shielded Tab quote', message: 'Testnet payment request' };
  q.c = commitment(q);
  const result = runZec(['request', '--address', q.address, '--amount-zat', q.amount_zat, '--session-id', q.session_id, '--nonce-hex', nonce, '--expiry-height', String(q.expiry_height), '--quote-id-hex', quote_id, '--label', q.label, '--message', q.message]);
  const quotes = await load(); quotes.push(q); await save(quotes);
  console.log(JSON.stringify({ quote: q, uri: result.uri }));
} else if (cmd === 'watch') {
  let scanText;
  if (opts.input) scanText = await readFile(opts.input, 'utf8');
  else {
    if (!process.env.STAB_UFVK_FILE || !opts.start) throw new Error('watch needs --input or STAB_UFVK_FILE and --start');
    const p = spawnSync(process.env.STAB_ZEC ?? '../target/debug/stab-zec', ['scan', '--ufvk-file', process.env.STAB_UFVK_FILE, '--start', opts.start, ...(opts.end ? ['--end', opts.end] : [])], { encoding: 'utf8' });
    if (p.status !== 0) throw new Error(p.stderr || p.stdout);
    scanText = p.stdout;
  }
  const lines = scanText.trim().split('\n').filter(Boolean).map(JSON.parse);
  const quotes = await load(), used = new Set();
  const tip = Number(opts.tip ?? opts.end);
  if (!Number.isSafeInteger(tip) || tip < 1) throw new Error('--tip or --end (current testnet height) is required');
  const adapter = opts.adapter === 'devnet' ? await devnetAdapter({ dryRun: args.includes('--dry-run') }) : new MockFlopAdapter(adapterFile);
  const dryRunCalls = [];
  for (const output of lines) for (const q of quotes) {
    const result = matchPayment(q, { ...output, session_id: q.session_id }, tip, used);
    if (result.ok) {
      const opened = await adapter.openSession({ owner: q.owner, delegate: q.delegate, c: result.c, sessionId: q.session_id, spendingCap: BigInt(q.spending_cap ?? 1), durationBlocks: Number(q.duration_blocks ?? 1000) });
      if (opened?.dryRun) dryRunCalls.push(opened);
      else { used.add(output.txid); used.add(result.memo.quote_id); q.used = output.txid; q.flop_block = opened.block_number; q.flop_block_hash = opened.block; }
    }
  }
  await save(quotes); await adapter.close?.(); console.log(JSON.stringify({ ok: true, used: [...used], dry_run_calls: dryRunCalls }));
} else if (cmd === 'verify') {
  const data = opts.input
    ? { outputs: (await readFile(opts.input, 'utf8')).trim().split('\n').filter(Boolean).map(JSON.parse) }
    : runZec(['decrypt-tx', '--ufvk-file', opts['viewing-key'], ...(opts['tx-hex-file'] ? ['--tx-hex-file', opts['tx-hex-file'], '--height', String(opts.height)] : ['--txid', opts.txid])]);
  let address = opts.address;
  if (!address) address = runZec(['ufvk-address', '--ufvk-file', opts['viewing-key']]).address;
  refuseMainnet(address);
  const parts = opts['session-id']?.split(':');
  if (parts?.length !== 4) throw new Error('valid --session-id is required');
  const outputs = data.outputs.filter(o => o.txid === opts.txid && o.stab?.c && o.stab?.nonce &&
    Number(o.height) <= Number(o.stab.expiry_height) && BigInt(o.value_zat) >= BigInt(o.stab.amount_zat) &&
    commitment({ network: 'test', address, amount_zat: o.stab.amount_zat, session_id: opts['session-id'], nonce: o.stab.nonce, expiry_height: Number(o.stab.expiry_height) }) === o.stab.c);
  const adapter = opts.adapter === 'devnet' ? await devnetAdapter() : new MockFlopAdapter(adapterFile);
  const paymentOk = outputs.length > 0;
  const flopOk = paymentOk && await adapter.verify({ owner: parts[2], delegate: parts[3], c: outputs[0].stab.c, sessionId: opts['session-id'], block: opts['flop-block'] });
  const ok = paymentOk && flopOk;
  await adapter.close?.();
  console.log(JSON.stringify({ ok, txid: opts.txid, session_id: opts['session-id'], commitment: outputs[0]?.stab.c, outputs: outputs.length, adapter: opts.adapter ?? 'mock' }));
  if (!ok) process.exitCode = 1;
} else if (cmd === 'test-fixture') {
  console.log(JSON.stringify({ ok: true, note: 'fixture adapter accepts JSONL supplied by the test harness' }));
} else {
  console.error('usage: stab-broker quote|watch|verify'); process.exitCode = 2;
}
