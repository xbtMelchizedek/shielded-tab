//! `stab-zec`: the CLI is a router. Each verb parses, calls the library, prints JSON, exits.
//! Expected refusals print `{"ok":false,"error":...}` and exit 1.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use serde_json::{json, Value};

use stab_zec::chain::{parse_txid, Source};
use stab_zec::commitment::commitment;
use stab_zec::decrypt::{decrypt_outputs, parse_tx};
use stab_zec::memo::parse_memo;
use stab_zec::network::DEFAULT_ENDPOINT;
use stab_zec::request::{build_request, parse_request, quote_fields, QuoteInput};
use stab_zec::scan::{trial_decrypt_block, ScanKeys};
use stab_zec::wallet::{default_address, keygen_testnet, read_ufvk_file};

type BoxErr = Box<dyn std::error::Error + Send + Sync>;

#[derive(Parser)]
#[command(name = "stab-zec", version, about = "Shielded Tab, Zcash testnet side (receive and scan only)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args)]
struct QuoteArgs {
    /// Broker receiving address (testnet UA or Sapling), exactly as it goes into the URI.
    #[arg(long)]
    address: String,
    #[arg(long)]
    amount_zat: u64,
    /// flop-devnet:<genesis>:<owner-ss58>:<delegate-ss58>
    #[arg(long)]
    session_id: String,
    /// 32 bytes, lowercase hex.
    #[arg(long)]
    nonce_hex: String,
    #[arg(long)]
    expiry_height: u32,
    /// 16 bytes, lowercase hex.
    #[arg(long)]
    quote_id_hex: String,
    #[arg(long)]
    label: Option<String>,
    #[arg(long)]
    message: Option<String>,
}

#[derive(Args)]
struct SourceArgs {
    /// lightwalletd gRPC endpoint (https only).
    #[arg(long, default_value = DEFAULT_ENDPOINT, conflicts_with = "fixture_dir")]
    endpoint: String,
    /// Offline chain fixture dir (chain.json, compact_blocks.hex, txs/).
    #[arg(long)]
    fixture_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build a ZIP-321 URI with a STAB0 memo for a quote; print URI, C and memo.
    Request(QuoteArgs),
    /// Compute C for quote fields (no URI).
    Commit(QuoteArgs),
    /// Parse a ZIP-321 URI and its STAB0 memo.
    ParseRequest {
        #[arg(long)]
        uri: String,
    },
    /// lightwalletd GetLightdInfo; refuses anything but chain "test".
    ChainInfo {
        #[command(flatten)]
        source: SourceArgs,
    },
    /// Stateless range scan: trial-decrypt compact blocks, then decrypt hits from full txs.
    /// Prints one JSON line per decrypted output.
    Scan {
        #[arg(long, env = "STAB_UFVK_FILE")]
        ufvk_file: PathBuf,
        #[command(flatten)]
        source: SourceArgs,
        #[arg(long)]
        start: u32,
        /// Inclusive; defaults to the chain tip.
        #[arg(long)]
        end: Option<u32>,
    },
    /// Decrypt one transaction (raw hex file + height, or txid fetched from a source).
    DecryptTx {
        #[arg(long, env = "STAB_UFVK_FILE")]
        ufvk_file: PathBuf,
        #[arg(long, conflicts_with = "txid", requires = "height")]
        tx_hex_file: Option<PathBuf>,
        #[arg(long)]
        height: Option<u32>,
        #[arg(long)]
        txid: Option<String>,
        #[command(flatten)]
        source: SourceArgs,
    },
    /// Generate a fresh TESTNET seed into a new 0600 file; print only UFVK and address.
    KeygenTestnet {
        #[arg(long)]
        seed_out: PathBuf,
    },
    /// Derive the default testnet unified address from a UFVK.
    UfvkAddress { #[arg(long, env = "STAB_UFVK_FILE")] ufvk_file: PathBuf },
}

fn quote_input(a: &QuoteArgs) -> Result<QuoteInput, BoxErr> {
    let nonce: [u8; 32] = decode_hex_exact(&a.nonce_hex, "nonce")?;
    let quote_id: [u8; 16] = decode_hex_exact(&a.quote_id_hex, "quote id")?;
    Ok(QuoteInput {
        recv_addr: a.address.clone(),
        amount_zat: a.amount_zat,
        session_id: a.session_id.clone(),
        nonce,
        expiry_height: a.expiry_height,
        quote_id,
        label: a.label.clone(),
        message: a.message.clone(),
    })
}

fn decode_hex_exact<const N: usize>(s: &str, what: &str) -> Result<[u8; N], BoxErr> {
    if s.len() != 2 * N || s.bytes().any(|b| !matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(format!("{what} must be {N} bytes of lowercase hex").into());
    }
    let mut out = [0u8; N];
    hex::decode_to_slice(s, &mut out)?;
    Ok(out)
}

async fn open_source(s: &SourceArgs) -> Result<(Source, Value), BoxErr> {
    let (src, info) = match &s.fixture_dir {
        Some(dir) => Source::open_dir(dir)?,
        None => Source::connect(&s.endpoint).await?,
    };
    Ok((src, serde_json::to_value(info)?))
}

async fn run(cmd: Cmd) -> Result<Option<Value>, BoxErr> {
    match cmd {
        Cmd::Request(a) => {
            let built = build_request(&quote_input(&a)?)?;
            Ok(Some(json!({"ok": true, "uri": built.uri, "c_hex": hex::encode(built.c), "memo_hex": hex::encode(built.memo.as_slice())})))
        }
        Cmd::Commit(a) => {
            let q = quote_input(&a)?;
            build_request(&q)?;
            Ok(Some(json!({"ok": true, "c_hex": hex::encode(commitment(&quote_fields(&q)))})))
        }
        Cmd::ParseRequest { uri } => {
            let p = parse_request(&uri)?;
            let stab = parse_memo(&p.memo)?;
            Ok(Some(json!({"ok": true, "address": p.recv_addr, "amount_zat": p.amount_zat, "memo_hex": hex::encode(p.memo.as_slice()), "stab": stab})))
        }
        Cmd::ChainInfo { source } => {
            let (_, info) = open_source(&source).await?;
            Ok(Some(json!({"ok": true, "chain": info})))
        }
        Cmd::Scan { ufvk_file, source, start, end } => {
            let ufvk = read_ufvk_file(&ufvk_file)?;
            let keys = ScanKeys::from_ufvk(&ufvk);
            let (mut src, _) = open_source(&source).await?;
            let end = match end {
                Some(e) => e,
                None => src.latest_height().await?,
            };
            let mut candidates = Vec::new();
            let mut blocks = 0u32;
            src.for_each_block(start, end, |b| {
                blocks += 1;
                candidates.extend(trial_decrypt_block(b, &keys)?);
                Ok(())
            })
            .await?;
            for c in &candidates {
                let (raw, height) = src.get_tx(&c.txid).await?;
                if height != c.height {
                    return Err(format!("tx {} height {height} disagrees with compact block {}", c.txid, c.height).into());
                }
                let tx = parse_tx(&raw, height, Some(&c.txid.to_string()))?;
                for o in decrypt_outputs(&tx, height, &ufvk) {
                    println!("{}", serde_json::to_string(&o)?);
                }
            }
            eprintln!("{}", json!({"scanned_blocks": blocks, "start": start, "end": end, "tx_hits": candidates.len()}));
            Ok(None)
        }
        Cmd::DecryptTx { ufvk_file, tx_hex_file, height, txid, source } => {
            let ufvk = read_ufvk_file(&ufvk_file)?;
            let (raw, height, expect) = match (tx_hex_file, txid) {
                (Some(path), None) => {
                    let raw = hex::decode(std::fs::read_to_string(path)?.trim())?;
                    (raw, height.ok_or("--height is required with --tx-hex-file")?, None)
                }
                (None, Some(t)) => {
                    let id = parse_txid(&t)?;
                    let (mut src, _) = open_source(&source).await?;
                    let (raw, h) = src.get_tx(&id).await?;
                    if height.is_some_and(|want| want != h) {
                        return Err(format!("tx mined at {h}, not at --height").into());
                    }
                    (raw, h, Some(t))
                }
                _ => return Err("give exactly one of --tx-hex-file or --txid".into()),
            };
            let tx = parse_tx(&raw, height, expect.as_deref())?;
            let outputs = decrypt_outputs(&tx, height, &ufvk);
            Ok(Some(json!({"ok": true, "txid": tx.txid().to_string(), "height": height, "outputs": outputs})))
        }
        Cmd::KeygenTestnet { seed_out } => {
            let (ufvk, address) = keygen_testnet(&seed_out)?;
            Ok(Some(json!({"ok": true, "network": "test", "ufvk": ufvk, "address": address})))
        }
        Cmd::UfvkAddress { ufvk_file } => {
            let ufvk = read_ufvk_file(&ufvk_file)?;
            Ok(Some(json!({"ok": true, "address": default_address(&ufvk)?})))
        }
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    match run(cli.cmd).await {
        Ok(Some(v)) => {
            println!("{v}");
            std::process::ExitCode::SUCCESS
        }
        Ok(None) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            println!("{}", json!({"ok": false, "error": e.to_string()}));
            std::process::ExitCode::FAILURE
        }
    }
}
