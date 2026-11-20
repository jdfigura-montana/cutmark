//! The control API token: two random UUIDs, kept as a generic credential in Windows Credential Manager.
#![allow(unsafe_code, reason = "Credential Manager is a Win32 API; each unsafe block says why it is sound")]

use thiserror::Error;
use windows::Win32::Foundation::ERROR_NOT_FOUND;
use windows::Win32::Security::Credentials::{CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredFree, CredReadW, CredWriteW};
use windows::core::{PCWSTR, PWSTR};

/// The name of the credential that holds the token.
pub const TARGET: &str = "Cutmark/recorder/api-token";

/// The length of a token in characters: two UUIDs of 32 hex digits.
const TOKEN_LEN: usize = 64;

/// Why the token could not be read or stored.
#[derive(Debug, Error)]
pub enum TokenError {
    /// Credential Manager refused to hand over the token.
    #[error("Windows Credential Manager could not read the API token: {0}")]
    Read(windows::core::Error),
    /// Credential Manager refused to store the token.
    #[error("Windows Credential Manager could not store the API token: {0}")]
    Write(windows::core::Error),
}

/// A new token: two UUID v4 values in simple form, joined, 64 hex characters.
pub fn generate() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

/// Whether `token` looks like one that [`generate`] makes.
fn is_valid(token: &str) -> bool {
    token.len() == TOKEN_LEN && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Compares two tokens without stopping at the first difference.
pub fn matches(expected: &str, given: &str) -> bool {
    let (expected, given) = (expected.as_bytes(), given.as_bytes());
    let mut difference = expected.len() ^ given.len();
    for (index, byte) in expected.iter().enumerate() {
        difference |= usize::from(*byte ^ given.get(index).copied().unwrap_or(0));
    }
    difference == 0
}

/// Reads the stored token, making and storing a new one on the first run or when the stored one is damaged.
pub fn load_or_create() -> Result<String, TokenError> {
    load_or_create_at(TARGET)
}

/// Replaces the stored token with a new one and returns it.
pub fn regenerate() -> Result<String, TokenError> {
    regenerate_at(TARGET)
}

/// Does the work of [`load_or_create`] for the credential called `target`.
fn load_or_create_at(target: &str) -> Result<String, TokenError> {
    match read_at(target)? {
        Some(token) if is_valid(&token) => Ok(token),
        _ => regenerate_at(target),
    }
}

/// Does the work of [`regenerate`] for the credential called `target`.
fn regenerate_at(target: &str) -> Result<String, TokenError> {
    let token = generate();
    write_at(target, &token)?;
    Ok(token)
}

/// The UTF-16 form of `text` with the closing NUL that Win32 wants.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Reads the generic credential called `target`; `None` when there is none or it is not text.
fn read_at(target: &str) -> Result<Option<String>, TokenError> {
    let name = wide(target);
    let mut found: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: `name` is NUL-terminated and outlives the call, and `found` is a valid place for the pointer to be written.
    let read = unsafe { CredReadW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None, &mut found) };
    if let Err(err) = read {
        return if err.code() == ERROR_NOT_FOUND.to_hresult() { Ok(None) } else { Err(TokenError::Read(err)) };
    }
    if found.is_null() {
        return Ok(None);
    }
    // SAFETY: on success `found` points to a credential that stays valid until CredFree. Its blob, when not null, holds
    // `CredentialBlobSize` readable bytes. The bytes are copied before the credential is freed, and it is freed once.
    let text = unsafe {
        let credential = &*found;
        let bytes = if credential.CredentialBlob.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(credential.CredentialBlob, credential.CredentialBlobSize as usize).to_vec()
        };
        CredFree(found.cast_const().cast());
        String::from_utf8(bytes).ok()
    };
    Ok(text)
}

/// Stores `token` as the generic credential called `target`, replacing any earlier one.
fn write_at(target: &str, token: &str) -> Result<(), TokenError> {
    let mut name = wide(target);
    let mut user = wide("cutmark");
    let mut blob = token.as_bytes().to_vec();
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(name.as_mut_ptr()),
        UserName: PWSTR(user.as_mut_ptr()),
        CredentialBlobSize: u32::try_from(blob.len()).unwrap_or(0),
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..CREDENTIALW::default()
    };
    // SAFETY: every pointer in `credential` points into `name`, `user` or `blob`, which are NUL-terminated or sized as
    // declared and outlive the call. CredWriteW only reads the structure.
    unsafe { CredWriteW(&credential, 0) }.map_err(TokenError::Write)
}

#[cfg(test)]
mod tests {
    use windows::Win32::Security::Credentials::CredDeleteW;

    use super::*;

    /// Removes a test credential.
    fn delete_at(target: &str) {
        let name = wide(target);
        // SAFETY: `name` is NUL-terminated and outlives the call.
        let _ = unsafe { CredDeleteW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None) };
    }

    #[test]
    fn a_token_is_two_uuids_of_64_hex_characters_and_never_repeats() {
        let first = generate();
        assert_eq!(first.len(), 64);
        assert!(is_valid(&first));
        assert_ne!(first, generate());
        assert!(!is_valid("short"));
        assert!(!is_valid(&"g".repeat(64)));
    }

    #[test]
    fn matches_needs_every_character_and_the_same_length() {
        let token = generate();
        assert!(matches(&token, &token));
        assert!(!matches(&token, &token[..63]));
        assert!(!matches(&token, &format!("{token}0")));
        assert!(!matches(&token, &generate()));
        assert!(!matches(&token, ""));
    }

    #[test]
    fn the_token_is_made_once_kept_and_replaced_on_request() -> Result<(), TokenError> {
        let target = format!("Cutmark/recorder/test-token-{}", std::process::id());
        delete_at(&target);
        assert_eq!(read_at(&target)?, None);
        let first = load_or_create_at(&target)?;
        assert!(is_valid(&first));
        assert_eq!(load_or_create_at(&target)?, first);
        let second = regenerate_at(&target)?;
        assert_ne!(second, first);
        assert_eq!(load_or_create_at(&target)?, second);
        delete_at(&target);
        Ok(())
    }
}
