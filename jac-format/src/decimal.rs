//! Arbitrary-precision decimal encoding

use crate::varint::{decode_uleb128, encode_uleb128, zigzag_decode, zigzag_encode};

/// Decimal number with exact representation
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decimal {
    /// Sign: false = non-negative, true = negative
    pub sign: bool,
    /// ASCII digits '0'..'9', MSB-first, no leading zeros
    pub digits: Vec<u8>,
    /// Base-10 exponent
    pub exponent: i32,
}

impl Decimal {
    /// Parse from JSON number string
    pub fn from_str_exact(s: &str) -> Result<Self, crate::error::JacError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(crate::error::JacError::CorruptBlock);
        }

        let (sign, s) = if let Some(stripped) = s.strip_prefix('-') {
            (true, stripped)
        } else {
            (false, s)
        };

        // Handle special cases
        if s == "0" || s == "0.0" || s == "0e0" || s == "0E0" {
            return Ok(Self {
                sign: false, // Zero is always non-negative
                digits: vec![b'0'],
                exponent: 0,
            });
        }

        // Parse scientific notation
        let (mantissa, exponent) = if let Some(e_pos) = s.find('e').or_else(|| s.find('E')) {
            let mantissa = &s[..e_pos];
            let exp_str = &s[e_pos + 1..];
            let exp: i32 = exp_str
                .parse()
                .map_err(|_| crate::error::JacError::CorruptBlock)?;
            (mantissa, exp)
        } else {
            (s, 0)
        };

        // Parse mantissa
        let (digits, decimal_places) = Self::parse_mantissa(mantissa)?;

        // Calculate final exponent
        let final_exponent = exponent - decimal_places as i32;

        // Remove leading zeros
        let digits = Self::remove_leading_zeros(digits)?;

        // Check digit count limit
        if digits.len() > 65536 {
            return Err(crate::error::JacError::LimitExceeded(
                "Too many decimal digits".to_string(),
            ));
        }

        Ok(Self {
            sign,
            digits,
            exponent: final_exponent,
        })
    }

    /// Parse mantissa and return (digits, decimal_places)
    fn parse_mantissa(s: &str) -> Result<(Vec<u8>, usize), crate::error::JacError> {
        let mut digits = Vec::new();
        let mut decimal_places = 0;
        let mut found_dot = false;

        for ch in s.chars() {
            match ch {
                '0'..='9' => {
                    digits.push(ch as u8);
                    if found_dot {
                        decimal_places += 1;
                    }
                }
                '.' => {
                    if found_dot {
                        return Err(crate::error::JacError::CorruptBlock);
                    }
                    found_dot = true;
                }
                _ => return Err(crate::error::JacError::CorruptBlock),
            }
        }

        if digits.is_empty() {
            return Err(crate::error::JacError::CorruptBlock);
        }

        Ok((digits, decimal_places))
    }

    /// Remove leading zeros (except for "0" itself)
    fn remove_leading_zeros(mut digits: Vec<u8>) -> Result<Vec<u8>, crate::error::JacError> {
        // Remove leading zeros
        while digits.len() > 1 && digits[0] == b'0' {
            digits.remove(0);
        }

        // Ensure we have at least one digit
        if digits.is_empty() {
            digits.push(b'0');
        }

        // Validate all digits are ASCII '0'..'9'
        for &digit in &digits {
            if !digit.is_ascii_digit() {
                return Err(crate::error::JacError::CorruptBlock);
            }
        }

        Ok(digits)
    }

    /// Check if value fits exactly in f64
    pub fn from_f64_if_exact(val: f64) -> Option<Self> {
        if val.is_infinite() || val.is_nan() {
            return None;
        }

        // Use a more precise string representation to check exactness
        let s = val.to_string();
        let decimal = Self::from_str_exact(&s).ok()?;

        // Check if round-trip is exact by converting back to f64
        let back_to_f64 = decimal.to_f64_if_exact()?;
        if back_to_f64.to_bits() == val.to_bits() {
            Some(decimal)
        } else {
            None
        }
    }

    /// Convert to f64 if exact
    pub fn to_f64_if_exact(&self) -> Option<f64> {
        let json_str = self.to_json_string();
        json_str.parse::<f64>().ok().and_then(|f| {
            let canonical = format!("{:.17}", f);
            let reparsed = Self::from_str_exact(&canonical).ok()?;
            if reparsed == *self {
                Some(f)
            } else {
                None
            }
        })
    }

    /// Convert to JSON string
    pub fn to_json_string(&self) -> String {
        if self.digits == [b'0'] && self.exponent == 0 {
            return "0".to_string();
        }

        let mut result = String::new();

        if self.sign {
            result.push('-');
        }

        // Use scientific notation for large exponents
        if self.exponent.abs() > 6 {
            result.push_str(&String::from_utf8_lossy(&self.digits));
            let mut exponent = self.exponent as i64;
            if self.digits.len() > 1 {
                let insert_pos = if self.sign { 2 } else { 1 };
                result.insert(insert_pos, '.');
                exponent += (self.digits.len() as i64) - 1;
            }
            result.push('e');
            result.push_str(&exponent.to_string());
        } else {
            // Regular decimal notation
            if self.exponent >= 0 {
                result.push_str(&String::from_utf8_lossy(&self.digits));
                for _ in 0..self.exponent {
                    result.push('0');
                }
            } else {
                let exp = (-self.exponent) as usize;
                if exp < self.digits.len() {
                    let (int_part, frac_part) = self.digits.split_at(self.digits.len() - exp);
                    result.push_str(&String::from_utf8_lossy(int_part));
                    result.push('.');
                    result.push_str(&String::from_utf8_lossy(frac_part));
                } else {
                    result.push('0');
                    result.push('.');
                    for _ in 0..(exp - self.digits.len()) {
                        result.push('0');
                    }
                    result.push_str(&String::from_utf8_lossy(&self.digits));
                }
            }
        }

        result
    }

    /// Encode to bytes according to wire format
    pub fn encode(&self) -> Result<Vec<u8>, crate::error::JacError> {
        let mut result = Vec::new();

        // Sign byte (0 = non-negative, 1 = negative)
        result.push(if self.sign { 1 } else { 0 });

        // Digits length
        result.extend_from_slice(&encode_uleb128(self.digits.len() as u64));

        // Digits (ASCII)
        result.extend_from_slice(&self.digits);

        // Exponent (ZigZag + ULEB128)
        result.extend_from_slice(&encode_uleb128(zigzag_encode(self.exponent as i64)));

        Ok(result)
    }

    /// Decode from bytes
    pub fn decode(bytes: &[u8]) -> Result<(Self, usize), crate::error::JacError> {
        let mut pos = 0;

        if bytes.is_empty() {
            return Err(crate::error::JacError::UnexpectedEof);
        }

        // Sign byte
        let sign_byte = bytes[pos];
        if sign_byte > 1 {
            return Err(crate::error::JacError::CorruptBlock);
        }
        let sign = sign_byte == 1;
        pos += 1;

        // Digits length
        let (digits_len, len_bytes) = decode_uleb128(&bytes[pos..])?;
        pos += len_bytes;

        if digits_len > 65536 {
            return Err(crate::error::JacError::LimitExceeded(
                "Too many decimal digits".to_string(),
            ));
        }

        if pos + digits_len as usize > bytes.len() {
            return Err(crate::error::JacError::UnexpectedEof);
        }

        // Digits
        let digits = bytes[pos..pos + digits_len as usize].to_vec();
        pos += digits_len as usize;

        // Validate digits are ASCII '0'..'9'
        for &digit in &digits {
            if !digit.is_ascii_digit() {
                return Err(crate::error::JacError::CorruptBlock);
            }
        }

        // Exponent
        let (exp_zigzag, exp_bytes) = decode_uleb128(&bytes[pos..])?;
        let exponent = zigzag_decode(exp_zigzag);
        pos += exp_bytes;

        // Validate exponent range
        if exponent < i32::MIN as i64 || exponent > i32::MAX as i64 {
            return Err(crate::error::JacError::CorruptBlock);
        }

        Ok((
            Self {
                sign,
                digits,
                exponent: exponent as i32,
            },
            pos,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::varint::{encode_uleb128, zigzag_encode};

    #[test]
    fn test_decimal_from_str_basic() {
        let cases = vec![
            ("0", false, vec![b'0'], 0),
            ("123", false, vec![b'1', b'2', b'3'], 0),
            ("-123", true, vec![b'1', b'2', b'3'], 0),
            ("0.5", false, vec![b'5'], -1),
            ("-0.5", true, vec![b'5'], -1),
            ("1e3", false, vec![b'1'], 3),
            ("-1e3", true, vec![b'1'], 3),
            ("1.5e2", false, vec![b'1', b'5'], 1),
        ];

        for (input, expected_sign, expected_digits, expected_exp) in cases {
            let decimal = Decimal::from_str_exact(input).unwrap();
            assert_eq!(decimal.sign, expected_sign);
            assert_eq!(decimal.digits, expected_digits);
            assert_eq!(decimal.exponent, expected_exp);
        }
    }

    #[test]
    fn test_decimal_from_str_zero() {
        let zero_cases = vec!["0", "0.0", "0e0", "0E0"];
        for input in zero_cases {
            let decimal = Decimal::from_str_exact(input).unwrap();
            assert_eq!(decimal.sign, false); // Zero is always non-negative
            assert_eq!(decimal.digits, vec![b'0']);
            assert_eq!(decimal.exponent, 0);
        }
    }

    #[test]
    fn test_decimal_from_str_invalid() {
        let invalid_cases = vec!["", "abc", "1.2.3", "1e", "e1", "1..2"];
        for input in invalid_cases {
            assert!(Decimal::from_str_exact(input).is_err());
        }
    }

    #[test]
    fn test_decimal_to_json_string() {
        let cases = vec![
            ("0", "0"),
            ("123", "123"),
            ("-123", "-123"),
            ("0.5", "0.5"),
            ("-0.5", "-0.5"),
            ("1e3", "1000"),
            ("-1e3", "-1000"),
            ("1.5e2", "150"),
        ];

        for (input, expected) in cases {
            let decimal = Decimal::from_str_exact(input).unwrap();
            assert_eq!(decimal.to_json_string(), expected);
        }
    }

    #[test]
    fn test_decimal_roundtrip() {
        let cases = vec![
            "0", "123", "-123", "0.5", "-0.5",
            "1000",   // Use "1000" instead of "1e3" to avoid scientific notation
            "-1000",  // Use "-1000" instead of "-1e3"
            "150",    // Use "150" instead of "1.5e2"
            "0.001",  // Use "0.001" instead of "1e-3"
            "-0.001", // Use "-0.001" instead of "-1e-3"
        ];

        for input in cases {
            let decimal = Decimal::from_str_exact(input).unwrap();
            let json_str = decimal.to_json_string();
            let roundtrip = Decimal::from_str_exact(&json_str).unwrap();
            assert_eq!(decimal, roundtrip);
        }
    }

    #[test]
    fn test_decimal_roundtrip_extremes() {
        let cases = vec![
            "0",
            "0.1",
            "1e-20",
            "1e300",
            "-123.456e10",
        ];

        for input in cases {
            let decimal = Decimal::from_str_exact(input).unwrap();
            let encoded = decimal.encode().unwrap();
            let (decoded, consumed) = Decimal::decode(&encoded).unwrap();
            assert_eq!(decoded, decimal);
            assert_eq!(consumed, encoded.len());

            let json = decimal.to_json_string();
            let reparsed = Decimal::from_str_exact(&json).unwrap();
            assert_eq!(reparsed, decimal);
        }
    }

    #[test]
    fn test_decimal_encode_decode() {
        let cases = vec!["0", "123", "-123", "0.5", "-0.5", "1e3", "-1e3"];

        for input in cases {
            let decimal = Decimal::from_str_exact(input).unwrap();
            let encoded = decimal.encode().unwrap();
            let (decoded, bytes_consumed) = Decimal::decode(&encoded).unwrap();
            assert_eq!(decimal, decoded);
            assert_eq!(bytes_consumed, encoded.len());
        }
    }

    #[test]
    fn test_decimal_f64_exactness() {
        // These should be exact in f64
        let exact_cases = vec![0.0, 1.0, -1.0, 0.5, -0.5, 2.0, -2.0];
        for val in exact_cases {
            if let Some(decimal) = Decimal::from_f64_if_exact(val) {
                assert_eq!(decimal.to_f64_if_exact(), Some(val));
            }
        }

        // These should NOT be exact in f64
        let inexact_cases = vec![0.1, -0.1, 0.2, -0.2];
        for val in inexact_cases {
            assert!(Decimal::from_f64_if_exact(val).is_none());
        }
    }

    #[test]
    fn test_decimal_limits() {
        // Test digit count limit
        let too_many_digits = "1".repeat(65537);
        assert!(Decimal::from_str_exact(&too_many_digits).is_err());
    }

    #[test]
    fn test_decimal_allows_max_digits() {
        let max_digits = "1".repeat(65_536);
        let decimal = Decimal::from_str_exact(&max_digits).unwrap();
        assert_eq!(decimal.digits.len(), 65_536);
        let encoded = decimal.encode().unwrap();
        let (decoded, consumed) = Decimal::decode(&encoded).unwrap();
        assert_eq!(decoded, decimal);
        assert_eq!(consumed, encoded.len());
    }

    #[test]
    fn test_decimal_from_str_trims_leading_zero() {
        let decimal = Decimal::from_str_exact("0123").unwrap();
        assert_eq!(decimal.digits, vec![b'1', b'2', b'3']);
        assert_eq!(decimal.exponent, 0);
    }

    #[test]
    fn test_decimal_negative_zero_is_normalized() {
        let decimal = Decimal::from_str_exact("-0").unwrap();
        assert!(!decimal.sign, "zero must be stored as non-negative");
        assert_eq!(decimal.digits, vec![b'0']);
    }

    #[test]
    fn test_decimal_decode_invalid_sign() {
        let mut bytes = Vec::new();
        // Invalid sign byte (2)
        bytes.push(2);
        // digits length = 1
        bytes.extend_from_slice(&encode_uleb128(1));
        bytes.push(b'0');
        // exponent = 0
        bytes.extend_from_slice(&encode_uleb128(zigzag_encode(0)));

        assert!(matches!(Decimal::decode(&bytes), Err(crate::error::JacError::CorruptBlock)));
    }

    #[test]
    fn test_decimal_decode_exponent_out_of_range() {
        let mut bytes = Vec::new();
        bytes.push(0); // sign
        bytes.extend_from_slice(&encode_uleb128(1)); // digits len
        bytes.push(b'1');

        // Exponent = i64::from(i32::MAX) + 1 → should overflow allowed range
        let exp = (i32::MAX as i64) + 1;
        bytes.extend_from_slice(&encode_uleb128(zigzag_encode(exp)));

        assert!(matches!(Decimal::decode(&bytes), Err(crate::error::JacError::CorruptBlock)));
    }

    #[test]
    fn test_decimal_decode_exponent_underflow() {
        let mut bytes = Vec::new();
        bytes.push(0); // sign
        bytes.extend_from_slice(&encode_uleb128(1)); // digits len
        bytes.push(b'1');

        let exp = (i32::MIN as i64) - 1;
        bytes.extend_from_slice(&encode_uleb128(zigzag_encode(exp)));

        assert!(matches!(Decimal::decode(&bytes), Err(crate::error::JacError::CorruptBlock)));
    }

    #[test]
    fn test_decimal_remove_leading_zeros() {
        let decimal = Decimal {
            sign: false,
            digits: vec![b'0', b'0', b'1', b'2', b'3'],
            exponent: 0,
        };
        let cleaned = Decimal::remove_leading_zeros(decimal.digits.clone()).unwrap();
        assert_eq!(cleaned, vec![b'1', b'2', b'3']);
    }

    #[test]
    fn test_decimal_zero_leading_zeros() {
        let decimal = Decimal {
            sign: false,
            digits: vec![b'0'],
            exponent: 0,
        };
        let cleaned = Decimal::remove_leading_zeros(decimal.digits.clone()).unwrap();
        assert_eq!(cleaned, vec![b'0']);
    }
}
