//! Compact-block trial decryption (Sapling + Orchard) with a UFVK's incoming viewing keys.
//! A hit only says "this transaction has an output for us"; values and memos come from
//! the full transaction (see `decrypt`).

use std::collections::BTreeSet;

use orchard::note_encryption::{CompactAction, OrchardDomain};
use sapling_crypto::note_encryption::{try_sapling_compact_note_decryption, CompactOutputDescription};
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_note_encryption::try_compact_note_decryption;
use zcash_primitives::transaction::components::sapling::zip212_enforcement;
use zcash_protocol::consensus::BlockHeight;
use zcash_protocol::TxId;
use zip32::Scope;

use crate::network::NETWORK;

/// Prepared incoming viewing keys, external and internal scope, per pool.
pub struct ScanKeys {
    sapling: Vec<sapling_crypto::keys::PreparedIncomingViewingKey>,
    orchard: Vec<orchard::keys::PreparedIncomingViewingKey>,
}

impl ScanKeys {
    pub fn from_ufvk(ufvk: &UnifiedFullViewingKey) -> Self {
        let scopes = [Scope::External, Scope::Internal];
        let sapling = ufvk
            .sapling()
            .map(|d| scopes.iter().map(|s| sapling_crypto::keys::PreparedIncomingViewingKey::new(&d.to_ivk(*s))).collect())
            .unwrap_or_default();
        let orchard = ufvk
            .orchard()
            .map(|f| scopes.iter().map(|s| orchard::keys::PreparedIncomingViewingKey::new(&f.to_ivk(*s))).collect())
            .unwrap_or_default();
        ScanKeys { sapling, orchard }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Candidate {
    pub height: u32,
    pub txid: TxId,
}

#[derive(Debug)]
pub enum ScanError {
    Height(u64),
    Txid,
    Malformed(&'static str),
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScanError::Height(h) => write!(f, "block height {h} out of range"),
            ScanError::Txid => write!(f, "compact tx id is not 32 bytes"),
            ScanError::Malformed(what) => write!(f, "malformed compact {what}"),
        }
    }
}

impl std::error::Error for ScanError {}

/// Trial-decrypt every shielded output in a compact block; return the txs with a hit.
/// A malformed compact output is an error, not a skip: the server is lying or broken.
pub fn trial_decrypt_block(block: &CompactBlock, keys: &ScanKeys) -> Result<Vec<Candidate>, ScanError> {
    let height = u32::try_from(block.height).map_err(|_| ScanError::Height(block.height))?;
    let zip212 = zip212_enforcement(&NETWORK, BlockHeight::from_u32(height));
    let mut hits = BTreeSet::new();
    for tx in &block.vtx {
        let txid_bytes: [u8; 32] = tx.txid.as_slice().try_into().map_err(|_| ScanError::Txid)?;
        let mut hit = false;
        for out in &tx.outputs {
            let out = CompactOutputDescription::try_from(out).map_err(|_| ScanError::Malformed("sapling output"))?;
            hit |= keys.sapling.iter().any(|ivk| try_sapling_compact_note_decryption(ivk, &out, zip212).is_some());
        }
        for act in &tx.actions {
            let act = CompactAction::try_from(act).map_err(|_| ScanError::Malformed("orchard action"))?;
            let domain = OrchardDomain::for_compact_action(&act);
            hit |= keys.orchard.iter().any(|ivk| try_compact_note_decryption(&domain, ivk, &act).is_some());
        }
        if hit {
            hits.insert(Candidate { height, txid: TxId::from_bytes(txid_bytes) });
        }
    }
    Ok(hits.into_iter().collect())
}
