import { readFile } from 'node:fs/promises';
import { connectFlopApi, getSession } from '@flop-labs/sdk';
import { Keyring } from '@polkadot/keyring';
import { blake2b } from '@noble/hashes/blake2.js';

export function buildBatch(api, { delegate, spendingCap, durationBlocks, c }) {
  const create = api.tx.flopSessionKeys.createSession(delegate, spendingCap, spendingCap, spendingCap, durationBlocks, []);
  const remarkBytes = Buffer.from(`STAB0:${c}`);
  const remark = api.tx.system.remarkWithEvent(`0x${remarkBytes.toString('hex')}`);
  const batch = api.tx.utility.batchAll([create, remark]);
  return { batch, remarkBytes };
}

export async function devnetAdapter({ dryRun = false } = {}) {
  const endpoint = process.env.FLOP_RPC ?? 'wss://rpc.devnet.flop.network';
  if (!endpoint.startsWith('wss://') || /mainnet/i.test(endpoint)) throw new Error('FLOP endpoint must be devnet WSS');
  const api = await connectFlopApi(endpoint);
  const genesis = api.genesisHash.toHex().slice(2).toLowerCase();
  if (genesis !== '99c172c095340ace980631f9123544c6bebd4839f3437e463533422d8be2d631') {
    await api.disconnect(); throw new Error(`FLOP genesis refused: ${genesis}`);
  }
  const adapter = {
    async openSession({ owner, delegate, c, spendingCap, durationBlocks }) {
      const { batch, remarkBytes } = buildBatch(api, { delegate, spendingCap, durationBlocks, c });
      if (dryRun) return { dryRun: true, genesis, call: batch.method.toHex() };
      const passPath = process.env.FLOP_KEYSTORE_PASS_FILE;
      const keyPath = process.env.FLOP_KEYSTORE;
      if (!keyPath || !passPath) throw new Error('FLOP_KEYSTORE and FLOP_KEYSTORE_PASS_FILE are required');
      const keyring = new Keyring({ type: 'sr25519' });
      const pair = keyring.addFromJson(JSON.parse(await readFile(keyPath, 'utf8')));
      pair.decodePkcs8((await readFile(passPath, 'utf8')).trim());
      if (pair.address !== owner) throw new Error('keystore owner does not match quote');
      const included = await new Promise((resolve, reject) => batch.signAndSend(pair, ({ status, dispatchError, events }) => {
        if (dispatchError) reject(new Error(dispatchError.toString()));
        else if (status.isInBlock || status.isFinalized) {
          if (events.some(({ event }) => event.section === 'system' && event.method === 'ExtrinsicFailed')) {
            reject(new Error('FLOP reported ExtrinsicFailed'));
            return;
          }
          const success = events.some(({ event }) => event.section === 'system' && event.method === 'ExtrinsicSuccess');
          const remarkHash = `0x${Buffer.from(blake2b(remarkBytes, { dkLen: 32 })).toString('hex')}`;
          const remarked = events.some(({ event }) => event.section === 'system' && event.method === 'Remarked' && event.data[0]?.toString() === owner && event.data[1]?.toHex?.() === remarkHash);
          if (!success || !remarked) reject(new Error('included FLOP batch lacks ExtrinsicSuccess or matching System.Remarked'));
          else resolve(status.asInBlock?.toHex() ?? status.asFinalized.toHex());
        }
      }).catch(reject));
      const block = await api.rpc.chain.getBlock(included);
      return { hash: batch.hash.toHex(), block: included, block_number: block.block.header.number.toNumber() };
    },
    async verify({ owner, delegate, c, block }) {
      if (block === undefined || !Number.isSafeInteger(Number(block))) throw new Error('--flop-block height is required for devnet verification');
      const s = await getSession(api, owner, delegate);
      if (!s) return false;
      const hash = await api.rpc.chain.getBlockHash(Number(block));
      const { block: data } = await api.rpc.chain.getBlock(hash);
      return data.extrinsics.some(tx => {
        if (tx.signer.toString() !== owner || tx.method.section !== 'utility' || tx.method.method !== 'batchAll') return false;
        const calls = tx.method.args[0];
        return calls.some(call => call.section === 'flopSessionKeys' && call.method === 'createSession' && call.args[0].toString() === delegate) &&
          calls.some(call => call.section === 'system' && call.method === 'remarkWithEvent' && call.args[0].toHex() === `0x${Buffer.from(`STAB0:${c}`).toString('hex')}`);
      });
    },
    close: () => api.disconnect(),
  };
  return adapter;
}
