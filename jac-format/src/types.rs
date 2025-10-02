//! Type tag enumeration

/// Type tag codes (3-bit)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TypeTag {
    /// Present but null
    Null = 0,
    /// Boolean value
    Bool = 1,
    /// Integer value
    Int = 2,
    /// Decimal value
    Decimal = 3,
    /// String value
    String = 4,
    /// Object (minified JSON)
    Object = 5,
    /// Array (minified JSON)
    Array = 6,
}

impl TypeTag {
    /// Convert from u8
    pub fn from_u8(val: u8) -> Result<Self, crate::error::JacError> {
        match val {
            0 => Ok(TypeTag::Null),
            1 => Ok(TypeTag::Bool),
            2 => Ok(TypeTag::Int),
            3 => Ok(TypeTag::Decimal),
            4 => Ok(TypeTag::String),
            5 => Ok(TypeTag::Object),
            6 => Ok(TypeTag::Array),
            7 => Err(crate::error::JacError::UnsupportedFeature(
                "Reserved type tag 7".to_string(),
            )),
            _ => Err(crate::error::JacError::UnsupportedFeature(format!(
                "Unknown type tag: {}",
                val
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_tag_from_u8_valid() {
        let cases = vec![
            (0, TypeTag::Null),
            (1, TypeTag::Bool),
            (2, TypeTag::Int),
            (3, TypeTag::Decimal),
            (4, TypeTag::String),
            (5, TypeTag::Object),
            (6, TypeTag::Array),
        ];

        for (val, expected) in cases {
            assert_eq!(TypeTag::from_u8(val).unwrap(), expected);
        }
    }

    #[test]
    fn test_type_tag_from_u8_reserved() {
        assert!(TypeTag::from_u8(7).is_err());
        if let Err(crate::error::JacError::UnsupportedFeature(msg)) = TypeTag::from_u8(7) {
            assert!(msg.contains("Reserved type tag 7"));
        }
    }

    #[test]
    fn test_type_tag_from_u8_invalid() {
        assert!(TypeTag::from_u8(8).is_err());
        assert!(TypeTag::from_u8(255).is_err());
    }

    #[test]
    fn test_type_tag_values() {
        assert_eq!(TypeTag::Null as u8, 0);
        assert_eq!(TypeTag::Bool as u8, 1);
        assert_eq!(TypeTag::Int as u8, 2);
        assert_eq!(TypeTag::Decimal as u8, 3);
        assert_eq!(TypeTag::String as u8, 4);
        assert_eq!(TypeTag::Object as u8, 5);
        assert_eq!(TypeTag::Array as u8, 6);
    }
}
