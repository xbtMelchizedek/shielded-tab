//! Full-transaction decryption with the official `decrypt_transaction`, plus STAB memo parse.

use std::collections::HashMap;

use serde::Serialize;
use zcash_client_backend::{decrypt_transaction, TransferType};
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::{BlockHeight, BranchId};
use zcash_protocol::memo::MemoBytes;

use crate::memo::{parse_memo, MemoError, StabMemo};
use crate::network::NETWORK;

#[derive(Debug, Clone, Serialize)]
pub struct DecryptedOutput {
    pub txid: String,
    pub height: u32,
    pub pool: &'static str,
    pub output_index: usize,
    /// "incoming" (paid to us), "internal" (our change), "outgoing" (recovered with our OVK).
    pub transfer: &'static str,
    pub value_zat: u64,
    pub memo_hex: String,
    pub stab: Option<StabMemo>,
    /// Set when the memo claims to be STAB but is malformed.
    pub stab_error: Option<String>,
}

#[derive(Debug)]
pub enum DecryptError {
    Parse(String),
    TxidMismatch { expected: String, actual: String },
    Unmined,
}

impl std::fmt::Display for DecryptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecryptError::Parse(e) => write!(f, "transaction parse: {e}"),
            DecryptError::TxidMismatch { expected, actual } => write!(f, "server returned tx {actual}, asked for {expected}"),
            DecryptError::Unmined => write!(f, "transaction is not mined (height unknown)"),
        }
    }
}

impl std::error::Error for DecryptError {}

fn transfer_name(t: TransferType) -> &'static str {
    match t {
        TransferType::Incoming => "incoming",
        TransferType::WalletInternal => "internal",
        TransferType::Outgoing => "outgoing",
    }
}

fn stab_fields(memo: &MemoBytes) -> (Option<StabMemo>, Option<String>) {
    match parse_memo(memo) {
        Ok(m) => (Some(m), None),
        Err(MemoError::NotStab) => (None, None),
        Err(e) => (None, Some(e.to_string())),
    }
}

/// Parse a raw transaction mined at `height`; refuse if its txid is not `expected_txid`
/// (display order hex) when one is given.
pub fn parse_tx(raw: &[u8], height: u32, expected_txid: Option<&str>) -> Result<Transaction, DecryptError> {
    if height == 0 {
        return Err(DecryptError::Unmined);
    }
    let branch = BranchId::for_height(&NETWORK, BlockHeight::from_u32(height));
    let tx = Transaction::read(raw, branch).map_err(|e| DecryptError::Parse(e.to_string()))?;
    let actual = tx.txid().to_string();
    if let Some(expected) = expected_txid {
        if expected != actual {
            return Err(DecryptError::TxidMismatch { expected: expected.into(), actual });
        }
    }
    Ok(tx)
}

/// Every output of `tx` the UFVK can decrypt, in pool then index order.
pub fn decrypt_outputs(tx: &Transaction, height: u32, ufvk: &UnifiedFullViewingKey) -> Vec<DecryptedOutput> {
    let ufvks = HashMap::from([(0u32, ufvk.clone())]);
    let d = decrypt_transaction(&NETWORK, Some(BlockHeight::from_u32(height)), None, tx, &ufvks);
    let txid = tx.txid().to_string();
    let mut out = Vec::new();
    for o in d.sapling_outputs() {
        let (stab, stab_error) = stab_fields(o.memo());
        out.push(DecryptedOutput {
            txid: txid.clone(),
            height,
            pool: "sapling",
            output_index: o.index(),
            transfer: transfer_name(o.transfer_type()),
            value_zat: o.note_value().into_u64(),
            memo_hex: hex::encode(o.memo().as_slice()),
            stab,
            stab_error,
        });
    }
    for o in d.orchard_outputs() {
        let (stab, stab_error) = stab_fields(o.memo());
        out.push(DecryptedOutput {
            txid: txid.clone(),
            height,
            pool: "orchard",
            output_index: o.index(),
            transfer: transfer_name(o.transfer_type()),
            value_zat: o.note_value().into_u64(),
            memo_hex: hex::encode(o.memo().as_slice()),
            stab,
            stab_error,
        });
    }
    out
}
