//! Reading secrets that Windows apps stash in Credential Manager.
//!
//! Claude Code normally writes `%USERPROFILE%\.claude\.credentials.json`, but
//! installs that opt into OS-backed storage keep the blob in the generic
//! credential store instead. We read it with `CredReadW`, which needs no
//! elevation for the current user's own credentials.

/// Fetch a generic credential's blob as a UTF-8 string.
///
/// Returns `None` when the credential does not exist, is not readable, or the
/// blob is not valid UTF-8. Never panics and never writes.
#[cfg(windows)]
pub fn read_generic_credential(target: &str) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Security::Credentials::{
        CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
    };

    let wide: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    let mut ptr: *mut CREDENTIALW = std::ptr::null_mut();

    // SAFETY: `wide` is NUL-terminated and outlives the call; on success the
    // returned buffer is handed straight back to CredFree.
    unsafe {
        if CredReadW(PCWSTR(wide.as_ptr()), CRED_TYPE_GENERIC, None, &mut ptr).is_err() {
            return None;
        }
        if ptr.is_null() {
            return None;
        }
        let cred = &*ptr;
        let bytes = if cred.CredentialBlob.is_null() || cred.CredentialBlobSize == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(cred.CredentialBlob, cred.CredentialBlobSize as usize)
                .to_vec()
        };
        CredFree(ptr as *const _);

        String::from_utf8(bytes).ok()
    }
}

/// Non-Windows builds have no credential store; adapters fall back to files.
#[cfg(not(windows))]
pub fn read_generic_credential(_target: &str) -> Option<String> {
    None
}

/// Credential Manager targets Claude Code is known to use, most specific first.
pub const CLAUDE_CREDENTIAL_TARGETS: &[&str] = &[
    "Claude Code-credentials",
    "Claude Code",
    "claude-code:credentials",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_credentials_are_none_not_a_panic() {
        // The point of the test on non-Windows hosts is that the stub compiles
        // and is safe to call; on Windows it exercises the real miss path.
        assert!(read_generic_credential("codenotch-definitely-not-a-real-target").is_none());
    }

    #[test]
    fn claude_targets_are_ordered_most_specific_first() {
        assert_eq!(CLAUDE_CREDENTIAL_TARGETS[0], "Claude Code-credentials");
    }
}
