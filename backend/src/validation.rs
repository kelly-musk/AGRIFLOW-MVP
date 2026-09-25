//! Small, shared input-validation helpers. Exists so the same rule (a field
//! must not be blank, an email must look like an email) is enforced the same
//! way everywhere it's needed, rather than each handler inventing its own
//! version of the check (see API_AUDIT.md R5 -- that inconsistency is what
//! this module fixes).

use crate::error::{AppError, AppResult};

pub fn require_non_empty(field: &str, value: &str) -> AppResult<()> {
    if value.trim().is_empty() {
        return Err(AppError::BadRequest(format!("{field} is required.")));
    }
    Ok(())
}

/// Deliberately permissive -- not RFC 5322 validation, just enough to reject
/// obviously-not-an-email input like `"not-an-email"`: exactly one `@`, a
/// non-empty local part, and a domain part containing at least one `.` with
/// non-empty labels on both sides of it.
pub fn is_valid_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else { return false };
    if local.is_empty() || domain.is_empty() || domain.contains('@') {
        return false;
    }
    let Some((label, tld)) = domain.rsplit_once('.') else { return false };
    !label.is_empty() && !tld.is_empty()
}

/// Deliberately permissive, same spirit as `is_valid_email` -- checks
/// shape (`0x` + 40 hex chars), not the EIP-55 mixed-case checksum. A
/// checksum mismatch is far more likely to be a wallet's display
/// preference than a typo, and rejecting valid-but-differently-cased
/// addresses would be more user-hostile than useful here.
pub fn is_valid_evm_address(address: &str) -> bool {
    let Some(hex) = address.strip_prefix("0x") else { return false };
    hex.len() == 40 && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_emails() {
        assert!(is_valid_email("buyer@kolafarms.com"));
        assert!(is_valid_email("a.b+c@sub.example.co"));
    }

    #[test]
    fn rejects_obviously_invalid_input() {
        assert!(!is_valid_email("not-an-email"));
        assert!(!is_valid_email("missing-domain@"));
        assert!(!is_valid_email("@missing-local.com"));
        assert!(!is_valid_email("two@at@signs.com"));
        assert!(!is_valid_email("no-dot@localhost"));
    }

    #[test]
    fn accepts_well_formed_evm_addresses() {
        assert!(is_valid_evm_address("0x9E93B3ffF884b736fECEACa33d93f33aAfDdc6C5"));
        assert!(is_valid_evm_address("0x0000000000000000000000000000000000000000"));
        // All-lowercase and all-uppercase hex bodies both accepted -- only
        // the "0x" prefix itself must stay lowercase, same as every real
        // address is actually written.
        assert!(is_valid_evm_address("0x41648de45cc4d0172becd4db0a0a0b459c383705"));
        assert!(is_valid_evm_address("0x41648DE45CC4D0172BECD4DB0A0A0B459C383705"));
    }

    #[test]
    fn rejects_malformed_evm_addresses() {
        assert!(!is_valid_evm_address("not-an-address"));
        assert!(!is_valid_evm_address("9E93B3ffF884b736fECEACa33d93f33aAfDdc6C5")); // missing 0x
        assert!(!is_valid_evm_address("0x9E93B3ffF884b736fECEACa33d93f33aAfDdc6")); // too short
        assert!(!is_valid_evm_address("0x9E93B3ffF884b736fECEACa33d93f33aAfDdc6C55")); // too long
        assert!(!is_valid_evm_address("0xZZ93B3ffF884b736fECEACa33d93f33aAfDdc6C5")); // non-hex
    }
}
