//! Read-only chain sources: a lightwalletd gRPC endpoint (TLS) or an offline fixture dir.
//! Both enforce the testnet gate before handing out any data.

use std::path::{Path, PathBuf};

use prost::Message;
use serde::{Deserialize, Serialize};
use tonic::transport::{Channel, ClientTlsConfig};
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_client_backend::proto::service::compact_tx_streamer_client::CompactTxStreamerClient;
use zcash_client_backend::proto::service::{BlockId, BlockRange, ChainSpec, Empty, TxFilter};
use zcash_protocol::TxId;

use crate::network::{check_chain_name, check_endpoint};

/// Hard ceiling on blocks per scan call (stateless scanner, no DB).
pub const MAX_SCAN_BLOCKS: u32 = 100_000;
/// Heights lightwalletd uses to mean "not mined in the main chain".
const UNMINED_HEIGHTS: [u64; 2] = [0, u64::MAX];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainInfo {
    pub chain_name: String,
    pub block_height: u64,
    pub sapling_activation_height: u64,
    pub consensus_branch_id: String,
    pub vendor: String,
    pub version: String,
}

/// One raw transaction in a fixture dir: `txs/<txid>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureTx {
    pub height: u32,
    pub raw_hex: String,
}

pub enum Source {
    Lwd(CompactTxStreamerClient<Channel>),
    Dir(PathBuf),
}

type BoxErr = Box<dyn std::error::Error + Send + Sync>;

fn err(msg: impl Into<String>) -> BoxErr {
    msg.into().into()
}

/// Txid in display (byte-reversed) hex, as explorers and `TxId::to_string` show it.
pub fn parse_txid(display_hex: &str) -> Result<TxId, BoxErr> {
    let mut bytes: [u8; 32] = hex::decode(display_hex)
        .map_err(|_| err("txid is not hex"))?
        .try_into()
        .map_err(|_| err("txid is not 32 bytes"))?;
    bytes.reverse();
    let txid = TxId::from_bytes(bytes);
    if txid.to_string() != display_hex {
        return Err(err("txid must be lowercase hex"));
    }
    Ok(txid)
}

impl Source {
    /// Connect over TLS and refuse unless lightwalletd says chain_name == "test".
    pub async fn connect(endpoint: &str) -> Result<(Source, ChainInfo), BoxErr> {
        let url = check_endpoint(endpoint)?;
        let channel = Channel::from_shared(url)?
            .tls_config(ClientTlsConfig::new().with_webpki_roots())?
            .connect()
            .await?;
        let mut client = CompactTxStreamerClient::new(channel);
        let info = client.get_lightd_info(Empty {}).await?.into_inner();
        check_chain_name(&info.chain_name)?;
        let info = ChainInfo {
            chain_name: info.chain_name,
            block_height: info.block_height,
            sapling_activation_height: info.sapling_activation_height,
            consensus_branch_id: info.consensus_branch_id,
            vendor: info.vendor,
            version: info.version,
        };
        Ok((Source::Lwd(client), info))
    }

    /// Offline source. `chain.json` must declare chain_name "test", same gate as lightwalletd.
    pub fn open_dir(dir: &Path) -> Result<(Source, ChainInfo), BoxErr> {
        let info: ChainInfo = serde_json::from_slice(&std::fs::read(dir.join("chain.json"))?)?;
        check_chain_name(&info.chain_name)?;
        Ok((Source::Dir(dir.to_path_buf()), info))
    }

    pub async fn latest_height(&mut self) -> Result<u32, BoxErr> {
        match self {
            Source::Lwd(c) => Ok(u32::try_from(c.get_latest_block(ChainSpec {}).await?.into_inner().height)?),
            Source::Dir(dir) => Ok(u32::try_from(Self::open_dir(dir)?.1.block_height)?),
        }
    }

    /// Visit compact blocks in [start, end] in height order.
    pub async fn for_each_block<F>(&mut self, start: u32, end: u32, mut visit: F) -> Result<(), BoxErr>
    where
        F: FnMut(&CompactBlock) -> Result<(), BoxErr>,
    {
        if end < start || end - start >= MAX_SCAN_BLOCKS {
            return Err(err(format!("scan range must be 1..={MAX_SCAN_BLOCKS} blocks")));
        }
        match self {
            Source::Lwd(c) => {
                let range = BlockRange {
                    start: Some(BlockId { height: start.into(), hash: vec![] }),
                    end: Some(BlockId { height: end.into(), hash: vec![] }),
                    pool_types: vec![],
                };
                let mut stream = c.get_block_range(range).await?.into_inner();
                let mut expect = u64::from(start);
                while let Some(block) = stream.message().await? {
                    if block.height != expect {
                        return Err(err(format!("server sent block {} where {expect} was expected", block.height)));
                    }
                    visit(&block)?;
                    expect += 1;
                }
                if expect != u64::from(end) + 1 {
                    return Err(err(format!("server stopped at {} before {end}", expect - 1)));
                }
            }
            Source::Dir(dir) => {
                let text = std::fs::read_to_string(dir.join("compact_blocks.hex")).unwrap_or_default();
                for line in text.lines().filter(|l| !l.trim().is_empty()) {
                    let block = CompactBlock::decode(hex::decode(line.trim())?.as_slice())?;
                    if (u64::from(start)..=u64::from(end)).contains(&block.height) {
                        visit(&block)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Raw transaction bytes and mined height. Unmined or forked transactions are refused.
    pub async fn get_tx(&mut self, txid: &TxId) -> Result<(Vec<u8>, u32), BoxErr> {
        let (raw, height) = match self {
            Source::Lwd(c) => {
                let filter = TxFilter { block: None, index: 0, hash: txid.as_ref().to_vec() };
                let r = c.get_transaction(filter).await?.into_inner();
                (r.data, r.height)
            }
            Source::Dir(dir) => {
                let path = dir.join("txs").join(format!("{txid}.json"));
                let t: FixtureTx = serde_json::from_slice(&std::fs::read(path).map_err(|_| err(format!("tx {txid} not in fixture dir")))?)?;
                (hex::decode(t.raw_hex)?, u64::from(t.height))
            }
        };
        if UNMINED_HEIGHTS.contains(&height) {
            return Err(err(format!("tx {txid} is not mined in the main chain")));
        }
        Ok((raw, u32::try_from(height)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn txid_is_display_order_and_exact() {
        let display = "0102030405060708091011121314151617181920212223242526272829303132";
        let txid = parse_txid(display).unwrap();
        assert_eq!(txid.as_ref()[0], 0x32);
        assert_eq!(txid.to_string(), display);
        assert!(parse_txid(&display.to_ascii_uppercase().replace("0102", "0A02")).is_err());
        assert!(parse_txid("abcd").is_err());
    }

    #[tokio::test]
    async fn mainnet_endpoint_is_refused_before_connecting() {
        let r = Source::connect("https://zec.rocks:443").await;
        assert!(r.err().unwrap().to_string().contains("mainnet endpoint refused"));
        let r = Source::connect("http://testnet.zec.rocks:443").await;
        assert!(r.err().unwrap().to_string().contains("https"));
    }

    #[test]
    fn fixture_dir_must_declare_testnet() {
        let dir = std::env::temp_dir().join(format!("stab-chain-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let info = ChainInfo { chain_name: "main".into(), block_height: 1, sapling_activation_height: 1, consensus_branch_id: String::new(), vendor: String::new(), version: String::new() };
        std::fs::write(dir.join("chain.json"), serde_json::to_vec(&info).unwrap()).unwrap();
        assert!(Source::open_dir(&dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
