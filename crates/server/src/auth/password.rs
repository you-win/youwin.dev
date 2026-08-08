//! argon2id hashing and verification.
//!
//! The hash lives in `YOUWIN_PASSWORD_HASH` as a PHC string. There is no user
//! table and no password column: one user, one secret, held by systemd's
//! `EnvironmentFile` at mode 0600.

use anyhow::{Result, anyhow};
use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use rand::Rng as _;

/// 16 bytes, the size argon2 recommends.
const SALT_BYTES: usize = 16;

// OWASP's baseline for argon2id: 19 MiB, two passes, one lane. The memory cost
// is the part that matters — it is what makes a stolen hash expensive to attack
// on a GPU.
const M_COST_KIB: u32 = 19_456;
const T_COST: u32 = 2;
const P_COST: u32 = 1;

fn hasher() -> Argon2<'static> {
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(M_COST_KIB, T_COST, P_COST, None).expect("compile-time constant params"),
    )
}

pub fn hash(password: &str) -> Result<String> {
    // Salt drawn from `rand`, not `SaltString::generate`. The latter wants an
    // RNG implementing rand_core 0.6's traits, while the rest of the crate is on
    // rand 0.10 — encoding our own bytes avoids carrying two RNG stacks for one
    // call site.
    let mut bytes = [0u8; SALT_BYTES];
    rand::rng().fill_bytes(&mut bytes);
    let salt = SaltString::encode_b64(&bytes).map_err(|e| anyhow!("encoding salt: {e}"))?;

    hasher()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| anyhow!("hashing password: {e}"))
}

/// Constant-time verification, delegated to the argon2 crate.
///
/// Returns `false` for a malformed PHC string rather than erroring: a corrupt
/// `YOUWIN_PASSWORD_HASH` should lock the door, not open it. Startup validates
/// the hash parses, so this branch means something changed underneath us.
pub fn verify(password: &str, phc: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(phc) else {
        tracing::error!("YOUWIN_PASSWORD_HASH is not a valid PHC string; refusing all logins");
        return false;
    };

    // Parameters come from the stored hash, not from `hasher()`, so raising the
    // constants above does not invalidate an existing password.
    hasher().verify_password(password.as_bytes(), &parsed).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_rejects_wrong_passwords() {
        let phc = hash("correct horse battery staple").unwrap();

        assert!(phc.starts_with("$argon2id$"), "{phc}");
        assert!(verify("correct horse battery staple", &phc));
        assert!(!verify("Correct horse battery staple", &phc));
        assert!(!verify("", &phc));
    }

    #[test]
    fn salts_differ_between_hashes_of_the_same_password() {
        assert_ne!(hash("same").unwrap(), hash("same").unwrap());
    }

    #[test]
    fn a_malformed_hash_denies_rather_than_admits() {
        for broken in ["", "not-a-phc-string", "$argon2id$v=19$garbage"] {
            assert!(!verify("anything", broken), "{broken:?} must not authenticate");
        }
    }
}
