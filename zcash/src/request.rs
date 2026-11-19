//! ZIP-321 payment request for a quote. The URI is parsed back with the zip321 crate
//! and every field compared before it is returned.

use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::Zatoshis;
use zip321::{Payment, TransactionRequest};

use crate::commitment::{commitment, QuoteFields};
use crate::memo::{encode_memo, parse_memo, StabMemo};
use crate::network::{parse_address, Refusal, NETWORK, NETWORK_NAME};
use crate::session::{parse_session_id, SessionIdError};

#[derive(Debug)]
pub enum RequestError {
    Network(Refusal),
    Session(SessionIdError),
    Amount,
    Zip321(String),
    RoundTrip(&'static str),
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestError::Network(r) => write!(f, "{r}"),
            RequestError::Session(e) => write!(f, "{e}"),
            RequestError::Amount => write!(f, "amount must be 1..=21e14 zatoshis"),
            RequestError::Zip321(e) => write!(f, "zip321: {e}"),
            RequestError::RoundTrip(field) => write!(f, "zip321 round trip changed {field}"),
        }
    }
}

impl std::error::Error for RequestError {}

#[derive(Debug, Clone)]
pub struct QuoteInput {
    pub recv_addr: String,
    pub amount_zat: u64,
    pub session_id: String,
    pub nonce: [u8; 32],
    pub expiry_height: u32,
    pub quote_id: [u8; 16],
    pub label: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BuiltRequest {
    pub uri: String,
    pub c: [u8; 32],
    pub memo: MemoBytes,
}

/// Validate the quote, compute C, build the memo and the ZIP-321 URI, and round-trip it.
pub fn build_request(q: &QuoteInput) -> Result<BuiltRequest, RequestError> {
    let addr = parse_address(&q.recv_addr).map_err(RequestError::Network)?;
    parse_session_id(&q.session_id).map_err(RequestError::Session)?;
    let amount = Zatoshis::from_u64(q.amount_zat).map_err(|_| RequestError::Amount)?;
    if amount == Zatoshis::ZERO {
        return Err(RequestError::Amount);
    }
    let c = commitment(&quote_fields(q));
    let memo = encode_memo(&StabMemo { c, nonce: q.nonce, quote_id: q.quote_id, amount_zat: q.amount_zat, expiry_height: q.expiry_height });
    let payment = Payment::new(
        addr.to_zcash_address(&NETWORK),
        Some(amount),
        Some(memo.clone()),
        q.label.clone(),
        q.message.clone(),
        vec![],
    )
    .map_err(|e| RequestError::Zip321(format!("{e:?}")))?;
    let uri = TransactionRequest::new(vec![payment])
        .map_err(|e| RequestError::Zip321(format!("{e}")))?
        .to_uri();
    check_round_trip(&uri, q, &memo)?;
    Ok(BuiltRequest { uri, c, memo })
}

pub fn quote_fields(q: &QuoteInput) -> QuoteFields {
    QuoteFields {
        network: NETWORK_NAME.into(),
        recv_addr: q.recv_addr.clone(),
        amount_zat: q.amount_zat,
        session_id: q.session_id.clone(),
        nonce: q.nonce,
        expiry_height: q.expiry_height,
    }
}

/// A single-payment request as parsed back from a URI.
#[derive(Debug, Clone)]
pub struct ParsedRequest {
    pub recv_addr: String,
    pub amount_zat: u64,
    pub memo: MemoBytes,
}

pub fn parse_request(uri: &str) -> Result<ParsedRequest, RequestError> {
    let req = TransactionRequest::from_uri(uri).map_err(|e| RequestError::Zip321(format!("{e}")))?;
    let payments = req.payments();
    let (Some(p), 1) = (payments.get(&0), payments.len()) else {
        return Err(RequestError::Zip321("expected exactly one payment at index 0".into()));
    };
    let recv_addr = p.recipient_address().encode();
    parse_address(&recv_addr).map_err(RequestError::Network)?;
    let amount_zat = p.amount().map(u64::from).ok_or(RequestError::Amount)?;
    let memo = p.memo().cloned().ok_or(RequestError::RoundTrip("memo"))?;
    Ok(ParsedRequest { recv_addr, amount_zat, memo })
}

fn check_round_trip(uri: &str, q: &QuoteInput, memo: &MemoBytes) -> Result<(), RequestError> {
    let parsed = parse_request(uri)?;
    if parsed.recv_addr != q.recv_addr {
        return Err(RequestError::RoundTrip("address"));
    }
    if parsed.amount_zat != q.amount_zat {
        return Err(RequestError::RoundTrip("amount"));
    }
    if parsed.memo.as_array() != memo.as_array() || parse_memo(&parsed.memo).is_err() {
        return Err(RequestError::RoundTrip("memo"));
    }
    let req = TransactionRequest::from_uri(uri).map_err(|e| RequestError::Zip321(format!("{e}")))?;
    let p = &req.payments()[&0];
    if p.label() != q.label.as_ref() || p.message() != q.message.as_ref() {
        return Err(RequestError::RoundTrip("label/message"));
    }
    Ok(())
}
