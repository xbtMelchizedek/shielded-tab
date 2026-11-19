//! Commitment C (SPEC.md section 3).
//!
//! C = BLAKE2b-256(personal = "ShieldedTab_C_v0", enc(fields)), where enc is the
//! concatenation of `u32_be(len) || bytes` for each field in this fixed order:
//! version, network, receiving address, amount_zat, session_id, nonce, expiry_height.

pub const PERSONALIZATION: &[u8; 16] = b"ShieldedTab_C_v0";
pub const VERSION: u8 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteFields {
    /// Network name; always "test" in v0.
    pub network: String,
    /// The receiving address exactly as it appears in the ZIP-321 request.
    pub recv_addr: String,
    pub amount_zat: u64,
    /// Canonical FLOP session id (see `session`).
    pub session_id: String,
    pub nonce: [u8; 32],
    /// Last Zcash block height at which a payment for this quote is accepted.
    pub expiry_height: u32,
}

fn push_field(out: &mut Vec<u8>, bytes: &[u8]) {
    let len = u32::try_from(bytes.len()).expect("field shorter than 4 GiB");
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(bytes);
}

/// The exact preimage bytes. Exposed so tests and the JS port can compare byte-for-byte.
pub fn encode_preimage(q: &QuoteFields) -> Vec<u8> {
    let mut out = Vec::with_capacity(256);
    push_field(&mut out, &[VERSION]);
    push_field(&mut out, q.network.as_bytes());
    push_field(&mut out, q.recv_addr.as_bytes());
    push_field(&mut out, &q.amount_zat.to_be_bytes());
    push_field(&mut out, q.session_id.as_bytes());
    push_field(&mut out, &q.nonce);
    push_field(&mut out, &q.expiry_height.to_be_bytes());
    out
}

pub fn commitment(q: &QuoteFields) -> [u8; 32] {
    let hash = blake2b_simd::Params::new()
        .hash_length(32)
        .personal(PERSONALIZATION)
        .to_state()
        .update(&encode_preimage(q))
        .finalize();
    let mut c = [0u8; 32];
    c.copy_from_slice(hash.as_bytes());
    c
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// SPEC.md test vector 1 (public test values; //Alice and //Bob dev accounts).
    pub fn vector1() -> QuoteFields {
        QuoteFields {
            network: "test".into(),
            recv_addr: "ztestsapling1vector0nly".into(),
            amount_zat: 150_000,
            session_id: "flop-devnet:99c172c095340ace980631f9123544c6bebd4839f3437e463533422d8be2d631:5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY:5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty".into(),
            nonce: [0x11; 32],
            expiry_height: 4_200_000,
        }
    }

    #[test]
    fn preimage_layout_is_length_prefixed() {
        let p = encode_preimage(&vector1());
        assert_eq!(&p[..5], &[0, 0, 0, 1, 0]);
        assert_eq!(&p[5..13], &[0, 0, 0, 4, b't', b'e', b's', b't']);
        let tail = &p[p.len() - 8..];
        assert_eq!(tail, &[0, 0, 0, 4, 0x00, 0x40, 0x16, 0x40]);
    }

    #[test]
    fn commitment_matches_spec_vector() {
        assert_eq!(hex::encode(commitment(&vector1())), "2cd62794f5b5bb726e0d2bfaebfb9b8a4f0ac007518e1a1693fff55efbd85bd3");
    }

    #[test]
    fn every_field_changes_c() {
        let base = commitment(&vector1());
        let mut variants: Vec<QuoteFields> = Vec::new();
        let mut v = vector1(); v.network = "main".into(); variants.push(v);
        let mut v = vector1(); v.recv_addr.push('x'); variants.push(v);
        let mut v = vector1(); v.amount_zat += 1; variants.push(v);
        let mut v = vector1(); v.session_id = v.session_id.replace("5FHne", "5FHnf"); variants.push(v);
        let mut v = vector1(); v.nonce[31] ^= 1; variants.push(v);
        let mut v = vector1(); v.expiry_height += 1; variants.push(v);
        for v in variants {
            assert_ne!(commitment(&v), base, "{v:?}");
        }
    }

    #[test]
    fn length_prefix_prevents_field_shifting() {
        // Moving a character between adjacent string fields must change C.
        let mut a = vector1();
        a.network = "tes".into();
        a.recv_addr = format!("t{}", vector1().recv_addr);
        assert_ne!(commitment(&a), commitment(&vector1()));
    }
}
