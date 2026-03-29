#[derive(Debug, Clone, PartialEq)]
pub enum DbValue {
    Null,
    Boolean(bool),
    Integer(i64),
    UnsignedInteger(u64),
    Float(f64),
    Decimal(String),
    Text(String),
    Binary(Vec<u8>),
    Date(String),
    DateTime(String),
    Time(String),
    Interval(String),
    Json(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryEncoding {
    Hex,
}

#[derive(Debug, Clone)]
pub struct ValueFormatter {
    binary_encoding: BinaryEncoding,
    null_representation: &'static str,
}

impl Default for ValueFormatter {
    fn default() -> Self {
        Self {
            binary_encoding: BinaryEncoding::Hex,
            null_representation: "",
        }
    }
}

impl ValueFormatter {
    pub fn format(&self, value: &DbValue) -> String {
        match value {
            DbValue::Null => self.null_representation.to_string(),
            DbValue::Boolean(value) => value.to_string(),
            DbValue::Integer(value) => value.to_string(),
            DbValue::UnsignedInteger(value) => value.to_string(),
            DbValue::Float(value) => value.to_string(),
            DbValue::Decimal(value)
            | DbValue::Text(value)
            | DbValue::Date(value)
            | DbValue::DateTime(value)
            | DbValue::Time(value)
            | DbValue::Interval(value)
            | DbValue::Json(value) => value.clone(),
            DbValue::Binary(bytes) => match self.binary_encoding {
                BinaryEncoding::Hex => hex_encode(bytes),
            },
        }
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);

    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }

    output
}

impl DbValue {
    pub fn from_str(s: &str, null_value: &str, type_hint: Option<&str>) -> anyhow::Result<Self> {
        if s == null_value || s.is_empty() {
            return Ok(DbValue::Null);
        }

        if let Some(type_name) = type_hint {
            return match type_name.to_lowercase().as_str() {
                "integer" | "int" | "bigint" | "smallint" => {
                    Ok(DbValue::Integer(s.parse()?))
                }
                "float" | "double" | "real" => {
                    Ok(DbValue::Float(s.parse()?))
                }
                "decimal" | "numeric" => {
                    Ok(DbValue::Decimal(s.to_string()))
                }
                "boolean" | "bool" => {
                    let b = matches!(s.to_lowercase().as_str(), "true" | "t" | "1" | "yes" | "y");
                    Ok(DbValue::Boolean(b))
                }
                "date" => {
                    Ok(DbValue::Date(s.to_string()))
                }
                "datetime" | "timestamp" => {
                    Ok(DbValue::DateTime(s.to_string()))
                }
                _ => Ok(DbValue::Text(s.to_string()))
            };
        }

        Ok(DbValue::Text(s.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::{DbValue, ValueFormatter};

    #[test]
    fn formatter_renders_binary_as_hex() {
        let formatter = ValueFormatter::default();

        assert_eq!(
            formatter.format(&DbValue::Binary(vec![0x00, 0xab, 0xff])),
            "00abff"
        );
    }

    #[test]
    fn formatter_renders_null_as_empty_string() {
        let formatter = ValueFormatter::default();

        assert_eq!(formatter.format(&DbValue::Null), "");
    }
}
