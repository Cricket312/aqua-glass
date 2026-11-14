//! Desktop secret storage adapter. Secrets (OAuth refresh tokens, the Google
//! client secret, the Last.fm key) go to the Windows Credential Manager. On
//! other desktop platforms (development only) they stay in memory for the
//! session (the portable default in `aqua_core::secrets`).
//!
//! The portable API (`get`/`set`/`delete`/`scrub`/`Redacted`/`keys`) lives in
//! `aqua_core::secrets` and is re-exported here.

pub use aqua_core::secrets::*;

#[cfg(windows)]
struct CredentialManager;

#[cfg(windows)]
impl SecretStore for CredentialManager {
    fn set(&self, key: &str, value: &str) -> anyhow::Result<()> {
        keyring::Entry::new(SERVICE, key)?.set_password(value)?;
        Ok(())
    }
    fn get(&self, key: &str) -> Option<String> {
        keyring::Entry::new(SERVICE, key).ok()?.get_password().ok()
    }
    fn delete(&self, key: &str) {
        if let Ok(e) = keyring::Entry::new(SERVICE, key) {
            let _ = e.delete_credential();
        }
    }
    fn persistent(&self) -> bool {
        true
    }
}

/// Install this platform's secret store (call once at startup).
pub fn install_platform_store() {
    #[cfg(windows)]
    aqua_core::secrets::install(Box::new(CredentialManager));
}
