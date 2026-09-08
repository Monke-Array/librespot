use crate::error::{Error, Result};

pub const JSON_SAFE_INTEGER_MAX: i64 = 9_007_199_254_740_991;

pub fn div_round_nearest_away(numerator: i64, denominator: i64) -> Result<i64> {
    if denominator == 0 {
        return Err(Error::new("INVALID_DIVISOR", "denominator must be nonzero"));
    }

    let numerator = i128::from(numerator);
    let denominator = i128::from(denominator);
    let mut quotient = numerator / denominator;
    let remainder = numerator % denominator;
    if remainder.abs() * 2 >= denominator.abs() {
        quotient += if numerator.signum() == denominator.signum() {
            1
        } else {
            -1
        };
    }

    i64::try_from(quotient)
        .map_err(|_| Error::new("INTEGER_OVERFLOW", "rounded quotient does not fit i64"))
}
