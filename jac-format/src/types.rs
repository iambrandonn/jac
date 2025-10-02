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
            7 => Err(crate::error::JacError::UnsupportedFeature("Reserved type tag 7".to_string())),
            _ => Err(crate::error::JacError::UnsupportedFeature(format!("Unknown type tag: {}", val))),
        }
    }
}

