//! Testnet key material for the broker's receiving wallet. The seed is written once to a
//! private file and dropped; the process only keeps the UFVK. No spending code exists.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use rand_core::{OsRng, RngCore};
use zcash_keys::keys::{UnifiedAddressRequest, UnifiedFullViewingKey, UnifiedSpendingKey};
use zip32::AccountId;

use crate::network::{parse_ufvk, NETWORK};

#[derive(Debug)]
pub enum KeygenError {
    Exists,
    InsideGitRepo,
    ParentNotPrivate,
    NoParent,
    Io(std::io::Error),
    Derive(String),
}

impl std::fmt::Display for KeygenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeygenError::Exists => write!(f, "seed file already exists; refusing to overwrite"),
            KeygenError::InsideGitRepo => write!(f, "seed path is inside a git work tree; refusing"),
            KeygenError::ParentNotPrivate => write!(f, "seed directory must not be group/world accessible (chmod 700)"),
            KeygenError::NoParent => write!(f, "seed path has no existing parent directory"),
            KeygenError::Io(e) => write!(f, "io: {}", e.kind()),
            KeygenError::Derive(e) => write!(f, "derivation: {e}"),
        }
    }
}

impl std::error::Error for KeygenError {}

/// UFVK for account 0 of a ZIP-32 seed on testnet. The spending key is a temporary.
pub fn ufvk_from_seed(seed: &[u8; 32]) -> Result<UnifiedFullViewingKey, KeygenError> {
    let usk = UnifiedSpendingKey::from_seed(&NETWORK, seed, AccountId::ZERO).map_err(|e| KeygenError::Derive(format!("{e:?}")))?;
    Ok(usk.to_unified_full_viewing_key())
}

/// Default shielded unified address (Orchard + Sapling receivers).
pub fn default_address(ufvk: &UnifiedFullViewingKey) -> Result<String, KeygenError> {
    let (ua, _) = ufvk.default_address(UnifiedAddressRequest::SHIELDED).map_err(|e| KeygenError::Derive(format!("{e:?}")))?;
    Ok(ua.encode(&NETWORK))
}

fn check_seed_path(path: &Path) -> Result<(), KeygenError> {
    if path.exists() {
        return Err(KeygenError::Exists);
    }
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let parent = parent.canonicalize().map_err(|_| KeygenError::NoParent)?;
    if parent.ancestors().any(|a| a.join(".git").exists()) {
        return Err(KeygenError::InsideGitRepo);
    }
    let mode = std::fs::metadata(&parent).map_err(KeygenError::Io)?.permissions().mode();
    if mode & 0o077 != 0 {
        return Err(KeygenError::ParentNotPrivate);
    }
    Ok(())
}

/// Generate a fresh 32-byte seed, write it as hex to `path` (mode 0600, create_new),
/// and return (UFVK, default address). The seed never leaves this function otherwise.
pub fn keygen_testnet(path: &Path) -> Result<(String, String), KeygenError> {
    check_seed_path(path)?;
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let ufvk = ufvk_from_seed(&seed)?;
    let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists { KeygenError::Exists } else { KeygenError::Io(e) }
    })?;
    let mut line = hex::encode(seed);
    line.push('\n');
    file.write_all(line.as_bytes()).map_err(KeygenError::Io)?;
    file.sync_all().map_err(KeygenError::Io)?;
    seed.fill(0);
    line.clear();
    let encoded = ufvk.encode(&NETWORK);
    debug_assert!(parse_ufvk(&encoded).is_ok());
    Ok((encoded, default_address(&ufvk)?))
}

/// Read a UFVK from a file (first non-empty, non-comment line). Errors never echo the contents.
pub fn read_ufvk_file(path: &Path) -> Result<UnifiedFullViewingKey, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read viewing key file: {}", e.kind()))?;
    let line = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#')).ok_or("viewing key file is empty")?;
    parse_ufvk(line).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{parse_address, Refusal};
    use zcash_protocol::consensus::{NetworkConstants, MAIN_NETWORK};

    /// TEST ONLY seed. Never fund anything derived from it.
    const TEST_SEED: [u8; 32] = [0x5a; 32];

    #[test]
    fn derived_keys_are_testnet() {
        let ufvk = ufvk_from_seed(&TEST_SEED).unwrap();
        let enc = ufvk.encode(&NETWORK);
        assert!(enc.starts_with("uviewtest1"), "{enc}");
        let ua = default_address(&ufvk).unwrap();
        assert!(ua.starts_with("utest1"), "{ua}");
        assert!(parse_address(&ua).is_ok());
        assert!(ufvk.sapling().is_some() && ufvk.orchard().is_some());
    }

    #[test]
    fn mainnet_encoding_of_same_key_is_refused() {
        let ufvk = ufvk_from_seed(&TEST_SEED).unwrap();
        let main = ufvk.encode(&MAIN_NETWORK);
        assert!(main.starts_with("uview1"));
        assert_eq!(parse_ufvk(&main).unwrap_err(), Refusal::MainnetKey);
        let (ua, _) = ufvk.default_address(UnifiedAddressRequest::SHIELDED).unwrap();
        assert_eq!(parse_address(&ua.encode(&MAIN_NETWORK)).unwrap_err(), Refusal::MainnetAddress);
        let (_, sapling) = ufvk.sapling().unwrap().default_address();
        let zs = zcash_keys::encoding::encode_payment_address(MAIN_NETWORK.hrp_sapling_payment_address(), &sapling);
        assert!(zs.starts_with("zs1"));
        assert_eq!(parse_address(&zs).unwrap_err(), Refusal::MainnetAddress);
        let zt = zcash_keys::encoding::encode_payment_address(NETWORK.hrp_sapling_payment_address(), &sapling);
        assert!(parse_address(&zt).is_ok());
        assert!(parse_address(&zt.to_ascii_uppercase()).is_err());
    }

    #[test]
    fn keygen_refuses_existing_file_and_git_trees() {
        let dir = std::env::temp_dir().join(format!("stab-keygen-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let seed_path = dir.join("seed.hex");
        let (ufvk, ua) = keygen_testnet(&seed_path).unwrap();
        assert!(ufvk.starts_with("uviewtest1") && ua.starts_with("utest1"));
        let mode = std::fs::metadata(&seed_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(matches!(keygen_testnet(&seed_path), Err(KeygenError::Exists)));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(keygen_testnet(&dir.join("b.hex")), Err(KeygenError::ParentNotPrivate)));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::create_dir(dir.join(".git")).unwrap();
        assert!(matches!(keygen_testnet(&dir.join("c.hex")), Err(KeygenError::InsideGitRepo)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
