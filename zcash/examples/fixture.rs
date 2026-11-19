#[cfg(not(test))]
use std::{fs, path::PathBuf};

use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
#[cfg(not(test))]
use serde_json::json;
use stab_zec::{memo::{encode_memo, StabMemo}, wallet::{default_address, ufvk_from_seed}};
#[cfg(not(test))]
use stab_zec::{decrypt::parse_tx, network::NETWORK};
use zcash_primitives::transaction::builder::{BuildConfig, Builder};
use zcash_protocol::{consensus::{BlockHeight, NetworkUpgrade, Parameters, TEST_NETWORK}, value::Zatoshis};
use transparent::{bundle::{OutPoint, TxOut}, builder::TransparentSigningSet, keys::{IncomingViewingKey, NonHardenedChildIndex}};

pub fn build_fixture() -> (Vec<u8>, String, u32, StabMemo) {
    build_fixture_with_memo(StabMemo { c: [0x31; 32], nonce: [0x42; 32], quote_id: [0x53; 16], amount_zat: 100_000, expiry_height: 4_200_000 })
}

pub fn build_fixture_with_memo(memo: StabMemo) -> (Vec<u8>, String, u32, StabMemo) {
    let seed = [7u8; 32]; // TEST ONLY; never fund or reuse outside tests.
    let ufvk = ufvk_from_seed(&seed).unwrap();
    let sapling_to = ufvk.sapling().unwrap().default_address().1;
    let orchard_to = ufvk.orchard().unwrap().address_at(0u32, orchard::keys::Scope::External);
    let height = TEST_NETWORK.activation_height(NetworkUpgrade::Nu5).unwrap().into();
    let mut builder = Builder::new(TEST_NETWORK, BlockHeight::from_u32(height), BuildConfig::Standard {
        sapling_anchor: Some(sapling_crypto::Anchor::empty_tree()),
        orchard_anchor: Some(orchard::Anchor::empty_tree()),
    });
    let tsk = transparent::keys::AccountPrivKey::from_seed(&TEST_NETWORK, &[9u8; 32], zip32::AccountId::ZERO).unwrap();
    let mut signing = TransparentSigningSet::new();
    let pubkey = signing.add_key(tsk.derive_external_secret_key(NonHardenedChildIndex::ZERO).unwrap());
    let prevout = TxOut::new(Zatoshis::const_from_u64(225_000), tsk.to_account_pubkey().derive_external_ivk().unwrap().derive_address(NonHardenedChildIndex::ZERO).unwrap().script().into());
    builder.add_transparent_p2pkh_input(pubkey, OutPoint::fake(), prevout).unwrap();
    builder.add_sapling_output::<std::convert::Infallible>(None, sapling_to, Zatoshis::const_from_u64(100_000), encode_memo(&memo)).unwrap();
    builder.add_orchard_output::<std::convert::Infallible>(None, orchard_to, Zatoshis::const_from_u64(100_000), encode_memo(&memo)).unwrap();
    let result = builder.mock_build(&signing, &[], &[], ChaCha20Rng::from_seed([0x64; 32])).unwrap();
    let mut raw = Vec::new();
    result.transaction().write(&mut raw).unwrap();
    (raw, default_address(&ufvk).unwrap(), height, memo)
}

#[cfg(not(test))]
fn main() {
    let path = std::env::args_os().nth(1).map(PathBuf::from).expect("fixture output path required");
    let memo = std::env::args_os().nth(2).map(|p| {
        let v: serde_json::Value = serde_json::from_slice(&fs::read(p).unwrap()).unwrap();
        StabMemo {
            c: hex::decode(v["c"].as_str().unwrap()).unwrap().try_into().unwrap(),
            nonce: hex::decode(v["nonce"].as_str().unwrap()).unwrap().try_into().unwrap(),
            quote_id: hex::decode(v["quote_id"].as_str().unwrap()).unwrap().try_into().unwrap(),
            amount_zat: v["amount_zat"].as_str().unwrap().parse().unwrap(),
            expiry_height: v["expiry_height"].as_u64().unwrap().try_into().unwrap(),
        }
    });
    let (raw, address, height, memo) = memo.map_or_else(build_fixture, build_fixture_with_memo);
    let tx = parse_tx(&raw, height, None).unwrap();
    let ufvk = ufvk_from_seed(&[7u8; 32]).unwrap();
    let value = json!({"tx_hex": hex::encode(raw), "txid": tx.txid().to_string(), "height": height, "address": address, "ufvk": ufvk.encode(&NETWORK), "memo": memo});
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}
