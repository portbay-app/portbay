//! Heap-zeroized SSH secrets.
//!
//! Passwords, key passphrases, and proxy passwords used to travel the connect
//! pipeline as plain `String`s, whose heap bytes survive `drop` until the
//! allocator happens to reuse them — visible in a crash dump or sysdiagnose.
//! [`SecretString`] zeroizes its buffer when the owning value drops, so every
//! place that *holds* a secret (keyring reads, prompted one-shot credentials,
//! the tunnel manager's reconnect copy) scrubs it at end of life. Borrowed
//! `&str` views into it (what the auth legs consume) add no copies.

/// An owned secret string whose heap allocation is zeroized on drop.
pub type SecretString = zeroize::Zeroizing<String>;

/// Borrow an optional secret as the `Option<&str>` the connect pipeline takes.
/// (`Option::as_deref` alone stops at `&String`, which doesn't coerce inside
/// the `Option`.)
pub fn secret_str(secret: &Option<SecretString>) -> Option<&str> {
    secret.as_ref().map(|s| s.as_str())
}

/// Wrap a freshly-deserialized (prompted) secret, trimming and discarding
/// blanks — the common "blank on edit / absent means fall back to keychain"
/// IPC convention. The incoming `String`'s buffer can't be retro-zeroized
/// (serde owns its allocation), but everything downstream of this boundary is.
pub fn nonblank_secret(s: Option<String>) -> Option<SecretString> {
    s.map(|v| SecretString::new(v.trim().to_string()))
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonblank_secret_trims_and_drops_blanks() {
        assert_eq!(nonblank_secret(None), None);
        assert_eq!(nonblank_secret(Some("".into())), None);
        assert_eq!(nonblank_secret(Some("   ".into())), None);
        assert_eq!(
            nonblank_secret(Some("  hunter2 ".into()))
                .as_deref()
                .map(String::as_str),
            Some("hunter2")
        );
    }

    #[test]
    fn secret_str_borrows_through_the_wrapper() {
        let secret = Some(SecretString::new("pw".into()));
        assert_eq!(secret_str(&secret), Some("pw"));
        assert_eq!(secret_str(&None), None);
    }
}
