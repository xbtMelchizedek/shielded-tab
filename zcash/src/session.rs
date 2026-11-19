//! FLOP devnet session id: `flop-devnet:<genesis-hex>:<owner-ss58>:<delegate-ss58>`.
//! Validation is strict and exact (SPEC.md section 2). No trimming, no case folding.

/// Devnet genesis hash (RPC `chain_getBlockHash(0)`, read 19 Nov 2026). Lowercase, no 0x.
pub const DEVNET_GENESIS_HEX: &str = "99c172c095340ace980631f9123544c6bebd4839f3437e463533422d8be2d631";
/// SS58 address format used by FLOP devnet (`api.registry.chainSS58`).
pub const DEVNET_SS58_PREFIX: u8 = 42;
const SESSION_TAG: &str = "flop-devnet";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionIdError {
    Shape,
    NotDevnet,
    GenesisHex,
    WrongGenesis,
    Ss58(&'static str),
    SameAccount,
}

impl std::fmt::Display for SessionIdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionIdError::Shape => write!(f, "session id must have 4 ':'-separated parts"),
            SessionIdError::NotDevnet => write!(f, "session id network tag must be flop-devnet"),
            SessionIdError::GenesisHex => write!(f, "genesis must be 64 lowercase hex chars without 0x"),
            SessionIdError::WrongGenesis => write!(f, "genesis is not the pinned FLOP devnet genesis"),
            SessionIdError::Ss58(why) => write!(f, "invalid ss58 account: {why}"),
            SessionIdError::SameAccount => write!(f, "owner and delegate must differ"),
        }
    }
}

impl std::error::Error for SessionIdError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionId {
    pub genesis_hex: String,
    pub owner: String,
    pub delegate: String,
}

/// Parse and validate a session id. Returns the parts; the canonical string is the input.
pub fn parse_session_id(s: &str) -> Result<SessionId, SessionIdError> {
    let parts: Vec<&str> = s.split(':').collect();
    let [tag, genesis, owner, delegate] = parts[..] else {
        return Err(SessionIdError::Shape);
    };
    if tag != SESSION_TAG {
        return Err(SessionIdError::NotDevnet);
    }
    if genesis.len() != 64 || !genesis.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(SessionIdError::GenesisHex);
    }
    if genesis != DEVNET_GENESIS_HEX {
        return Err(SessionIdError::WrongGenesis);
    }
    check_ss58(owner)?;
    check_ss58(delegate)?;
    if owner == delegate {
        return Err(SessionIdError::SameAccount);
    }
    Ok(SessionId { genesis_hex: genesis.into(), owner: owner.into(), delegate: delegate.into() })
}

/// SS58 with a one-byte prefix 42 and a 32-byte account id:
/// base58(prefix || id || blake2b_512("SS58PRE" || prefix || id)[..2]).
pub fn check_ss58(addr: &str) -> Result<[u8; 32], SessionIdError> {
    let raw = bs58::decode(addr).into_vec().map_err(|_| SessionIdError::Ss58("not base58"))?;
    if raw.len() != 35 {
        return Err(SessionIdError::Ss58("length"));
    }
    if raw[0] != DEVNET_SS58_PREFIX {
        return Err(SessionIdError::Ss58("prefix is not 42"));
    }
    let digest = blake2b_simd::Params::new().hash_length(64).to_state().update(b"SS58PRE").update(&raw[..33]).finalize();
    if digest.as_bytes()[..2] != raw[33..] {
        return Err(SessionIdError::Ss58("checksum"));
    }
    if bs58::encode(&raw).into_string() != addr {
        return Err(SessionIdError::Ss58("not canonical"));
    }
    let mut id = [0u8; 32];
    id.copy_from_slice(&raw[1..33]);
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Well-known public dev accounts (Substrate //Alice, //Bob), SS58 prefix 42.
    const ALICE: &str = "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY";
    const BOB: &str = "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty";

    fn sid(g: &str, o: &str, d: &str) -> String {
        format!("flop-devnet:{g}:{o}:{d}")
    }

    #[test]
    fn valid_session_id_parses() {
        let s = parse_session_id(&sid(DEVNET_GENESIS_HEX, ALICE, BOB)).unwrap();
        assert_eq!(s.owner, ALICE);
        assert_eq!(s.delegate, BOB);
    }

    #[test]
    fn session_id_is_exact() {
        let upper = DEVNET_GENESIS_HEX.to_ascii_uppercase();
        assert_eq!(parse_session_id(&sid(&upper, ALICE, BOB)), Err(SessionIdError::GenesisHex));
        assert_eq!(parse_session_id(&sid(&format!("0x{}", &DEVNET_GENESIS_HEX[2..]), ALICE, BOB)), Err(SessionIdError::GenesisHex));
        let other = format!("{}0", &DEVNET_GENESIS_HEX[..63]);
        assert_eq!(parse_session_id(&sid(&other, ALICE, BOB)), Err(SessionIdError::WrongGenesis));
        let s = sid(DEVNET_GENESIS_HEX, ALICE, BOB).replacen("flop-devnet", "flop-mainnet", 1);
        assert_eq!(parse_session_id(&s), Err(SessionIdError::NotDevnet));
        assert_eq!(parse_session_id(&format!("{} ", sid(DEVNET_GENESIS_HEX, ALICE, BOB))), Err(SessionIdError::Ss58("not base58")));
        assert_eq!(parse_session_id(&sid(DEVNET_GENESIS_HEX, ALICE, ALICE)), Err(SessionIdError::SameAccount));
        assert_eq!(parse_session_id("flop-devnet:x:y"), Err(SessionIdError::Shape));
    }

    #[test]
    fn ss58_checksum_and_prefix_are_checked() {
        let mut flipped = BOB.to_string();
        flipped.replace_range(10..11, if &BOB[10..11] == "a" { "b" } else { "a" });
        assert!(check_ss58(&flipped).is_err());
        // Polkadot (prefix 0) encoding of //Alice.
        assert_eq!(check_ss58("15oF4uVJwmo4TdGW7VfQxNLavjCXviqxT9S1MgbjMNHr6Sp5"), Err(SessionIdError::Ss58("prefix is not 42")));
    }

    #[test] fn rejects_empty_owner() { assert!(parse_session_id(&sid(DEVNET_GENESIS_HEX, "", BOB)).is_err()); }
    #[test] fn rejects_empty_delegate() { assert!(parse_session_id(&sid(DEVNET_GENESIS_HEX, ALICE, "")).is_err()); }
    #[test] fn rejects_trailing_session_field() { assert!(parse_session_id(&format!("{}:extra", sid(DEVNET_GENESIS_HEX, ALICE, BOB))).is_err()); }
    #[test] fn rejects_wrong_ss58_checksum() { assert!(check_ss58("5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQX").is_err()); }
    #[test] fn rejects_noncanonical_ss58() { assert!(check_ss58(&format!(" {ALICE}")).is_err()); }
}
