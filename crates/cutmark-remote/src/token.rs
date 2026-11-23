//! The per-launch security token: 32 random bytes as hex (spec: Remote Control "Security").

/// The hex digits for the token text.
const HEX: &[u8; 16] = b"0123456789abcdef";

/// Generates a 64-character hex token from 32 random bytes.
pub(crate) fn generate() -> String {
    let mut bytes = [0u8; 32];
    if getrandom::fill(&mut bytes).is_err() {
        // Unreachable on every platform the suite ships on; a time-and-pid mix keeps
        // the server startable on a broken entropy source instead of panicking.
        let mut seed = u64::from(std::process::id()) ^ system_nanos();
        for byte in &mut bytes {
            seed = seed.wrapping_mul(0x0100_0000_01b3).wrapping_add(0xcbf2_9ce4_8422_2325);
            *byte = (seed >> 33) as u8;
        }
    }
    let mut text = String::with_capacity(64);
    for byte in bytes {
        text.push(HEX[usize::from(byte >> 4)] as char);
        text.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    text
}

/// Nanoseconds since the Unix epoch, or 0 when the clock is before it.
fn system_nanos() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |span| u64::try_from(span.as_nanos()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_is_64_hex_characters() {
        let token = generate();
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(generate(), generate());
    }
}
