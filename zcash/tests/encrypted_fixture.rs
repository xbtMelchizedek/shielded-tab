#[path = "../examples/fixture.rs"]
mod fixture;

use std::{fs, process::Command};
use stab_zec::{decrypt::{decrypt_outputs, parse_tx}, network::{parse_ufvk, NETWORK}, scan::{trial_decrypt_block, ScanKeys}, wallet::ufvk_from_seed};
use zcash_client_backend::proto::compact_formats::{CompactBlock, CompactTx, CompactSaplingOutput, CompactOrchardAction};
use zcash_protocol::consensus::MAIN_NETWORK;

fn temp_dir() -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("stab-fixture-{}-{}", std::process::id(), std::thread::current().name().unwrap_or("test")));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn compact_block(raw: &[u8], height: u32) -> CompactBlock {
    let tx = parse_tx(raw, height, None).unwrap();
    let mut ctx = CompactTx { txid: tx.txid().as_ref().to_vec(), ..Default::default() };
    let sapling = tx.sapling_bundle().unwrap();
    ctx.outputs = sapling.shielded_outputs().iter().map(CompactSaplingOutput::from).collect();
    let orchard = tx.orchard_bundle().unwrap();
    ctx.actions = orchard.actions().iter().map(CompactOrchardAction::from).collect();
    CompactBlock { height: height.into(), vtx: vec![ctx], ..Default::default() }
}

#[test]
fn real_v5_fixture_decrypts_both_pools_and_scans_compact_outputs() {
    let (raw, address, height, memo) = fixture::build_fixture();
    let ufvk = ufvk_from_seed(&[7u8; 32]).unwrap();
    let tx = parse_tx(&raw, height, None).unwrap();
    assert_eq!(tx.version(), zcash_primitives::transaction::TxVersion::V5);
    let outputs = decrypt_outputs(&tx, height, &ufvk);
    assert_eq!(outputs.len(), 2);
    for (output, pool) in outputs.iter().zip(["sapling", "orchard"]) {
        assert_eq!(output.pool, pool);
        assert_eq!(output.transfer, "incoming");
        assert_eq!(output.value_zat, 100_000);
        assert_eq!(stab_zec::memo::parse_memo(&zcash_protocol::memo::MemoBytes::from_bytes(&hex::decode(&output.memo_hex).unwrap()).unwrap()).unwrap(), memo);
        assert_eq!(output.stab.as_ref(), Some(&memo));
    }
    assert!(address.starts_with("utest1"));

    let block = compact_block(&raw, height);
    let hits = trial_decrypt_block(&block, &ScanKeys::from_ufvk(&ufvk)).unwrap();
    assert_eq!(hits.len(), 1);
    let mut sapling_only = block.clone();
    sapling_only.vtx[0].actions.clear();
    assert_eq!(trial_decrypt_block(&sapling_only, &ScanKeys::from_ufvk(&ufvk)).unwrap().len(), 1);
    let mut orchard_only = block.clone();
    orchard_only.vtx[0].outputs.clear();
    assert_eq!(trial_decrypt_block(&orchard_only, &ScanKeys::from_ufvk(&ufvk)).unwrap().len(), 1);
    let other = ufvk_from_seed(&[8u8; 32]).unwrap();
    assert!(trial_decrypt_block(&block, &ScanKeys::from_ufvk(&other)).unwrap().is_empty());
}

#[test]
fn binary_decrypts_fixture_and_refuses_mainnet_ufvk() {
    let (raw, _, height, _) = fixture::build_fixture();
    let ufvk = ufvk_from_seed(&[7u8; 32]).unwrap();
    let dir = temp_dir();
    let key_path = dir.join("ufvk.txt");
    let tx_path = dir.join("tx.hex");
    fs::write(&key_path, ufvk.encode(&NETWORK)).unwrap();
    fs::write(&tx_path, hex::encode(raw)).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_stab-zec")).args(["decrypt-tx", "--ufvk-file"]).arg(&key_path).args(["--tx-hex-file"]).arg(&tx_path).args(["--height", &height.to_string()]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["outputs"].as_array().unwrap().len(), 2);
    let mainnet = ufvk.encode(&MAIN_NETWORK);
    assert!(parse_ufvk(&mainnet).is_err());
    fs::write(&key_path, mainnet).unwrap();
    let refused = Command::new(env!("CARGO_BIN_EXE_stab-zec")).args(["decrypt-tx", "--ufvk-file"]).arg(&key_path).args(["--tx-hex-file"]).arg(&tx_path).args(["--height", &height.to_string()]).output().unwrap();
    assert!(!refused.status.success());
    fs::remove_dir_all(dir).unwrap();
}
