//! Validated public inputs for the CoinFlip demo.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer};

pub fn validate_bet(bet: u8) -> Result<u8, &'static str> {
    match bet {
        0 | 1 => Ok(bet),
        _ => Err("bet must be 0 (heads) or 1 (tails)"),
    }
}

/// A positive STRK amount represented exactly as an integer number of wei.
/// Accept decimal strings or JSON numbers; never multiply an f64 by 10^18.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrkAmount(u128);

impl StrkAmount {
    pub fn wei(self) -> u128 {
        self.0
    }
}

impl FromStr for StrkAmount {
    type Err = &'static str;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.trim();
        let (mantissa, exponent) = match input.split_once(['e', 'E']) {
            Some((m, e)) => (m, e.parse::<i32>().map_err(|_| "invalid amount exponent")?),
            None => (input, 0),
        };
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        if whole.is_empty()
            || !whole.bytes().all(|b| b.is_ascii_digit())
            || !fraction.bytes().all(|b| b.is_ascii_digit())
            || mantissa.ends_with('.')
        {
            return Err("amount must be a positive decimal STRK value");
        }
        let digits = format!("{whole}{fraction}");
        let significant = digits.trim_start_matches('0');
        if significant.is_empty() {
            return Err("amount must be greater than zero");
        }
        let scale = 18 + i64::from(exponent) - fraction.len() as i64;
        let wei = if scale >= 0 {
            if scale > 38 {
                return Err("amount exceeds the supported wei range");
            }
            significant
                .parse::<u128>()
                .ok()
                .and_then(|n| n.checked_mul(10u128.pow(scale as u32)))
                .ok_or("amount exceeds the supported wei range")?
        } else {
            let keep = significant
                .len()
                .checked_sub((-scale) as usize)
                .ok_or("amount must be an exact number of wei")?;
            if significant[keep..].bytes().any(|b| b != b'0') {
                return Err("amount must be an exact number of wei");
            }
            significant[..keep]
                .parse::<u128>()
                .map_err(|_| "amount exceeds the supported wei range")?
        };
        Ok(Self(wei))
    }
}

impl fmt::Display for StrkAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let whole = self.0 / 10u128.pow(18);
        let fraction = self.0 % 10u128.pow(18);
        if fraction == 0 {
            write!(f, "{whole}")
        } else {
            let fraction = format!("{fraction:018}");
            write!(f, "{whole}.{}", fraction.trim_end_matches('0'))
        }
    }
}

impl<'de> Deserialize<'de> for StrkAmount {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Decimal {
            Text(String),
            Number(serde_json::Number),
        }
        let text = match Decimal::deserialize(deserializer)? {
            Decimal::Text(text) => text,
            Decimal::Number(number) => number.to_string(),
        };
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_are_exact_at_wei_precision_and_u128_boundary() {
        for (input, wei) in [
            ("0.001", 1_000_000_000_000_000),
            ("0.100000000000000001", 100_000_000_000_000_001),
            ("0.000000000000000001", 1),
            ("1e-18", 1),
            ("1e2", 100_000_000_000_000_000_000),
            ("0.100000000000000000000", 100_000_000_000_000_000),
            ("340282366920938463463.374607431768211455", u128::MAX),
        ] {
            let amount: StrkAmount = input.parse().unwrap();
            assert_eq!(amount.wei(), wei, "{input}");
            assert_eq!(amount.to_string().parse::<StrkAmount>().unwrap(), amount);
        }
    }

    #[test]
    fn invalid_zero_sub_wei_and_overflow_amounts_are_rejected() {
        for input in [
            "",
            "0",
            "-1",
            "+1",
            "NaN",
            "inf",
            "Infinity",
            "1.1.1",
            "1.",
            ".1",
            "0.0000000000000000001",
            "1e-19",
            "1e999999999",
            "1e-2147483648",
            "340282366920938463463.374607431768211456",
            "1e39",
        ] {
            assert!(input.parse::<StrkAmount>().is_err(), "{input}");
        }
    }

    #[test]
    fn json_numbers_and_strings_preserve_decimal_precision() {
        for json in ["0.100000000000000001", "\"0.100000000000000001\""] {
            let amount: StrkAmount = serde_json::from_str(json).unwrap();
            assert_eq!(amount.wei(), 100_000_000_000_000_001);
        }
        assert_eq!(
            serde_json::from_str::<StrkAmount>("1e-18").unwrap().wei(),
            1
        );
        for json in ["-1", "0", "null", "true", "1e-19", "\"NaN\""] {
            assert!(serde_json::from_str::<StrkAmount>(json).is_err(), "{json}");
        }
    }

    #[test]
    fn bets_are_never_silently_clamped() {
        assert_eq!(validate_bet(0), Ok(0));
        assert_eq!(validate_bet(1), Ok(1));
        assert!(validate_bet(2).is_err());
        assert!(validate_bet(u8::MAX).is_err());
    }
}
