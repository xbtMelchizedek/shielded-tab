//! Network gate. Zcash testnet only: every key, address, endpoint and lightwalletd
//! chain name passes through here before it is used. See SPEC.md section 7.

use zcash_keys::address::Address;
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_protocol::consensus::{TestNetwork, MAIN_NETWORK, TEST_NETWORK};

/// The only network this crate accepts.
pub const NETWORK: TestNetwork = TEST_NETWORK;
/// Network name as bound into the commitment and as reported by lightwalletd `chain_name`.
pub const NETWORK_NAME: &str = "test";
/// Default read-only lightwalletd endpoint (TLS).
pub const DEFAULT_ENDPOINT: &str = "https://testnet.zec.rocks:443";

/// Bech32/Bech32m human-readable parts of mainnet keys and addresses.
const MAINNET_HRPS: &[&str] = &["u", "zs", "uview", "uivk", "zxviews", "zivks", "tex", "secret-extended-key-main"];
/// Base58 prefixes of mainnet transparent addresses (P2PKH, P2SH).
const MAINNET_T_PREFIXES: &[&str] = &["t1", "t3"];
/// Hosts known to serve Zcash mainnet lightwalletd.
const MAINNET_HOSTS: &[&str] = &["mainnet.lightwalletd.com", "zec.rocks", "na.zec.rocks", "eu.zec.rocks", "ap.zec.rocks", "sa.zec.rocks"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    MainnetKey,
    MainnetAddress,
    MainnetEndpoint(String),
    EndpointNotTls,
    EndpointMalformed,
    WrongChain(String),
    KeyInvalid,
    AddressInvalid,
    AddressNotCanonical,
    AddressNoMemo,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::MainnetKey => write!(f, "mainnet viewing key refused: testnet only"),
            Refusal::MainnetAddress => write!(f, "mainnet address refused: testnet only"),
            Refusal::MainnetEndpoint(h) => write!(f, "mainnet endpoint refused: {h}"),
            Refusal::EndpointNotTls => write!(f, "endpoint must be https://"),
            Refusal::EndpointMalformed => write!(f, "endpoint is not https://host[:port]"),
            Refusal::WrongChain(c) => write!(f, "lightwalletd chain_name {c:?} refused: expected \"test\""),
            Refusal::KeyInvalid => write!(f, "not a testnet unified full viewing key"),
            Refusal::AddressInvalid => write!(f, "not a testnet Zcash address"),
            Refusal::AddressNotCanonical => write!(f, "address is not in canonical (lowercase) encoding"),
            Refusal::AddressNoMemo => write!(f, "address cannot receive a memo (transparent)"),
        }
    }
}

impl std::error::Error for Refusal {}

/// Bech32 human-readable part: everything before the last '1'.
fn hrp(s: &str) -> Option<&str> {
    s.rsplit_once('1').map(|(h, _)| h)
}

fn is_mainnet_encoding(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    if MAINNET_T_PREFIXES.iter().any(|p| s.starts_with(p)) {
        return true;
    }
    hrp(&lower).is_some_and(|h| MAINNET_HRPS.contains(&h))
}

/// Decode a testnet UFVK. Mainnet encodings are refused by name, not just by failure to parse.
pub fn parse_ufvk(encoded: &str) -> Result<UnifiedFullViewingKey, Refusal> {
    let s = encoded.trim();
    if is_mainnet_encoding(s) || UnifiedFullViewingKey::decode(&MAIN_NETWORK, s).is_ok() {
        return Err(Refusal::MainnetKey);
    }
    UnifiedFullViewingKey::decode(&NETWORK, s).map_err(|_| Refusal::KeyInvalid)
}

/// Validate a receiving address: testnet, shielded (can take a memo), canonical encoding.
/// The returned string is the exact text bound into the commitment.
pub fn parse_address(encoded: &str) -> Result<Address, Refusal> {
    if is_mainnet_encoding(encoded) || Address::decode(&MAIN_NETWORK, encoded).is_some() {
        return Err(Refusal::MainnetAddress);
    }
    let addr = Address::decode(&NETWORK, encoded).ok_or(Refusal::AddressInvalid)?;
    if addr.encode(&NETWORK) != encoded {
        return Err(Refusal::AddressNotCanonical);
    }
    if !addr.to_zcash_address(&NETWORK).can_receive_memo() {
        return Err(Refusal::AddressNoMemo);
    }
    Ok(addr)
}

/// Accept only `https://host[:port]` with no path, and not a known mainnet host.
pub fn check_endpoint(url: &str) -> Result<String, Refusal> {
    let rest = url.strip_prefix("https://").ok_or(Refusal::EndpointNotTls)?;
    let authority = rest.strip_suffix('/').unwrap_or(rest);
    if authority.is_empty() || authority.contains(['/', '@', '?', '#']) {
        return Err(Refusal::EndpointMalformed);
    }
    let host = authority.rsplit_once(':').map_or(authority, |(h, _)| h).to_ascii_lowercase();
    if MAINNET_HOSTS.contains(&host.as_str()) || host.contains("mainnet") {
        return Err(Refusal::MainnetEndpoint(host));
    }
    Ok(format!("https://{authority}"))
}

/// lightwalletd reports "main" or "test"; anything but "test" stops the command.
pub fn check_chain_name(chain_name: &str) -> Result<(), Refusal> {
    if chain_name == NETWORK_NAME {
        Ok(())
    } else {
        Err(Refusal::WrongChain(chain_name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_requires_tls_and_refuses_mainnet() {
        assert_eq!(check_endpoint("https://testnet.zec.rocks:443").unwrap(), "https://testnet.zec.rocks:443");
        assert_eq!(check_endpoint("http://testnet.zec.rocks:443"), Err(Refusal::EndpointNotTls));
        assert_eq!(check_endpoint("https://zec.rocks:443"), Err(Refusal::MainnetEndpoint("zec.rocks".into())));
        assert_eq!(check_endpoint("https://NA.ZEC.ROCKS:443"), Err(Refusal::MainnetEndpoint("na.zec.rocks".into())));
        assert_eq!(check_endpoint("https://mainnet.lightwalletd.com:9067"), Err(Refusal::MainnetEndpoint("mainnet.lightwalletd.com".into())));
        assert_eq!(check_endpoint("https://my-mainnet-node:443"), Err(Refusal::MainnetEndpoint("my-mainnet-node".into())));
        assert_eq!(check_endpoint("https://user@testnet.zec.rocks"), Err(Refusal::EndpointMalformed));
        assert_eq!(check_endpoint("https://testnet.zec.rocks/x"), Err(Refusal::EndpointMalformed));
    }

    #[test]
    fn chain_name_must_be_exactly_test() {
        assert!(check_chain_name("test").is_ok());
        for bad in ["main", "Test", "testnet", "regtest", ""] {
            assert!(check_chain_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn mainnet_prefixes_are_refused_before_decoding() {
        for s in ["u1qqqq", "zs1qqqq", "t1abc", "t3abc", "tex1qqqq", "uview1qqqq", "uivk1qqqq", "zxviews1qqqq"] {
            assert!(is_mainnet_encoding(s), "{s}");
        }
        for s in ["utest1qqqq", "ztestsapling1qqqq", "tmabc", "uviewtest1qqqq"] {
            assert!(!is_mainnet_encoding(s), "{s}");
        }
        assert_eq!(parse_address("zs1qqqq").unwrap_err(), Refusal::MainnetAddress);
        assert_eq!(parse_ufvk("uview1qqqq").unwrap_err(), Refusal::MainnetKey);
    }
}
