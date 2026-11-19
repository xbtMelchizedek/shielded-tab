//! STAB0 memo (SPEC.md section 4): a ZIP-302 text memo
//! `STAB0:<C hex 64>:<nonce hex 64>:<quote id hex 32>:<amount zat>:<expiry height>` padded to 512 bytes.

use serde::Serialize;
use zcash_protocol::memo::MemoBytes;

pub const MAGIC: &str = "STAB0";
/// Any memo starting with this is treated as a STAB memo of some version.
pub const MAGIC_FAMILY: &[u8] = b"STAB";
/// Byte length of the text before zero padding.
pub const MAX_TEXT_LEN: usize = 5 + 1 + 64 + 1 + 64 + 1 + 32 + 1 + 20 + 1 + 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StabMemo {
    #[serde(with = "hex_arr")]
    pub c: [u8; 32],
    #[serde(with = "hex_arr")]
    pub nonce: [u8; 32],
    #[serde(with = "hex_arr")]
    pub quote_id: [u8; 16],
    pub amount_zat: u64,
    pub expiry_height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoError {
    /// Not a STAB memo at all (empty, other text, binary). Not an error for a wallet.
    NotStab,
    /// Starts with the magic but violates the format. The broker must not guess.
    Malformed(&'static str),
}

impl std::fmt::Display for MemoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoError::NotStab => write!(f, "not a STAB0 memo"),
            MemoError::Malformed(why) => write!(f, "malformed STAB0 memo: {why}"),
        }
    }
}

impl std::error::Error for MemoError {}

pub fn encode_memo(m: &StabMemo) -> MemoBytes {
    let text = format!("{MAGIC}:{}:{}:{}:{}:{}", hex::encode(m.c), hex::encode(m.nonce), hex::encode(m.quote_id), m.amount_zat, m.expiry_height);
    debug_assert!(text.len() <= MAX_TEXT_LEN);
    MemoBytes::from_bytes(text.as_bytes()).expect("text shorter than 512 bytes")
}

fn lower_hex<const N: usize>(s: &[u8]) -> Result<[u8; N], MemoError> {
    if s.len() != 2 * N || !s.iter().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(MemoError::Malformed("field is not lowercase hex of the right length"));
    }
    let mut out = [0u8; N];
    hex::decode_to_slice(s, &mut out).map_err(|_| MemoError::Malformed("hex"))?;
    Ok(out)
}

/// Strict parse. A bounded UTF-8 text memo followed only by NUL padding.
pub fn parse_memo(memo: &MemoBytes) -> Result<StabMemo, MemoError> {
    let bytes = memo.as_array();
    if !bytes.starts_with(MAGIC_FAMILY) {
        return Err(MemoError::NotStab);
    }
    let text_len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let (text, pad) = bytes.split_at(text_len);
    if text_len > MAX_TEXT_LEN { return Err(MemoError::Malformed("text too long")); }
    if pad.iter().any(|&b| b != 0) {
        return Err(MemoError::Malformed("trailing bytes after expiry"));
    }
    let parts: Vec<&[u8]> = text.split(|&b| b == b':').collect();
    let [magic, c, nonce, quote_id, amount, expiry] = parts[..] else {
        return Err(MemoError::Malformed("expected 6 ':'-separated fields"));
    };
    if magic != MAGIC.as_bytes() {
        return Err(MemoError::Malformed("unsupported STAB version"));
    }
    fn decimal(s: &[u8]) -> Result<&str, MemoError> {
        if s.is_empty() || (s.len() > 1 && s[0] == b'0') || !s.iter().all(u8::is_ascii_digit) { return Err(MemoError::Malformed("invalid decimal")); }
        std::str::from_utf8(s).map_err(|_| MemoError::Malformed("invalid decimal"))
    }
    let amount_zat = decimal(amount)?.parse().map_err(|_| MemoError::Malformed("amount out of range"))?;
    let expiry_height = decimal(expiry)?.parse().map_err(|_| MemoError::Malformed("expiry out of range"))?;
    Ok(StabMemo { c: lower_hex(c)?, nonce: lower_hex(nonce)?, quote_id: lower_hex(quote_id)?, amount_zat, expiry_height })
}

mod hex_arr {
    pub fn serialize<S: serde::Serializer, const N: usize>(v: &[u8; N], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> StabMemo {
        StabMemo { c: [0xab; 32], nonce: [0x11; 32], quote_id: [0x22; 16], amount_zat: 0, expiry_height: 0 }
    }

    fn memo_from(text: &[u8]) -> MemoBytes {
        MemoBytes::from_bytes(text).unwrap()
    }

    #[test]
    fn round_trip() {
        let m = sample();
        let bytes = encode_memo(&m);
        assert_eq!(bytes.as_array()[0], b'S', "first byte <= 0xF4: ZIP-302 text memo");
        assert_eq!(parse_memo(&bytes).unwrap(), m);
    }

    #[test]
    fn golden_encoding() {
        let bytes = encode_memo(&sample());
        let expect = format!("STAB0:{}:{}:{}:0:0", "ab".repeat(32), "11".repeat(32), "22".repeat(16));
        assert_eq!(&bytes.as_array()[..expect.len()], expect.as_bytes());
        assert!(bytes.as_array()[expect.len()..].iter().all(|&b| b == 0));
    }

    #[test]
    fn non_stab_memos_are_not_errors() {
        assert_eq!(parse_memo(&MemoBytes::empty()), Err(MemoError::NotStab));
        assert_eq!(parse_memo(&memo_from(b"thanks for lunch")), Err(MemoError::NotStab));
        let mut arb = [0u8; 512];
        arb[0] = 0xFF;
        assert_eq!(parse_memo(&memo_from(&arb)), Err(MemoError::NotStab));
    }

    #[test] fn rejects_leading_zero_amount() { let s = format!("STAB0:{}:{}:{}:010:20", "ab".repeat(32), "11".repeat(32), "22".repeat(16)); assert!(parse_memo(&memo_from(s.as_bytes())).is_err()); }
    #[test] fn rejects_leading_zero_expiry() { let s = format!("STAB0:{}:{}:{}:10:020", "ab".repeat(32), "11".repeat(32), "22".repeat(16)); assert!(parse_memo(&memo_from(s.as_bytes())).is_err()); }
    #[test] fn rejects_amount_overflow() { let s = format!("STAB0:{}:{}:{}:18446744073709551616:20", "ab".repeat(32), "11".repeat(32), "22".repeat(16)); assert!(parse_memo(&memo_from(s.as_bytes())).is_err()); }
    #[test] fn rejects_expiry_overflow() { let s = format!("STAB0:{}:{}:{}:10:4294967296", "ab".repeat(32), "11".repeat(32), "22".repeat(16)); assert!(parse_memo(&memo_from(s.as_bytes())).is_err()); }
    #[test] fn rejects_empty_decimal() { let s = format!("STAB0:{}:{}:{}::20", "ab".repeat(32), "11".repeat(32), "22".repeat(16)); assert!(parse_memo(&memo_from(s.as_bytes())).is_err()); }

    #[test]
    fn malformed_stab_memos_are_rejected() {
        let good = format!("STAB0:{}:{}:{}:100:200", "ab".repeat(32), "11".repeat(32), "22".repeat(16));
        let cases = [
            good.to_ascii_uppercase().replace("STAB0", "STAB0"),
            format!("{good}x"),
            good.replacen(':', ";", 2),
            good.replace(":100:", ":0100:"),
            format!("STAB1{}", &good[5..]),
            good.replace("ab", "gg"),
        ];
        for text in cases {
            let r = parse_memo(&memo_from(text.as_bytes()));
            assert!(matches!(r, Err(MemoError::Malformed(_))), "{text}: {r:?}");
        }
        let mut stray = [0u8; 512];
        stray[..good.len()].copy_from_slice(good.as_bytes());
        stray[511] = 1;
        assert!(matches!(parse_memo(&memo_from(&stray)), Err(MemoError::Malformed(_))));
    }
}
