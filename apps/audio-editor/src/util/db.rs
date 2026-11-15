//! util::db: dB <-> linear conversion.

/// Converts decibels to a linear amplitude factor (1.0 at 0 dB).
pub fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Converts a linear amplitude factor to decibels.
/// Returns negative infinity for zero.
pub fn linear_to_db(lin: f32) -> f32 {
    if lin == 0.0 {
        return -f32::INFINITY;
    }
    20.0 * lin.log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_db_is_unity() {
        assert!((db_to_linear(0.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn minus_six_db_is_about_half() {
        assert!((db_to_linear(-6.0206) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn zero_linear_is_negative_infinity() {
        assert_eq!(linear_to_db(0.0), -f32::INFINITY);
    }

    #[test]
    fn round_trip_within_tolerance() {
        for db in [-60.0f32, -12.0, -3.0, -0.5, 0.0, 3.0, 12.0] {
            let lin = db_to_linear(db);
            let back = linear_to_db(lin);
            assert!((back - db).abs() < 1e-4, "round trip failed for {db}: got {back}");
        }
    }
}
