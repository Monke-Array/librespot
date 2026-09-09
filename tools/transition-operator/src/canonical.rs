use crate::error::{Error, Result};
use crate::scalar::JSON_SAFE_INTEGER_MAX;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let value = serde_json::to_value(value).map_err(|_| {
        Error::new(
            "JSON_SERIALIZATION_FAILED",
            "typed JSON serialization failed",
        )
    })?;
    let mut bytes = Vec::new();
    write_value(&value, &mut bytes)?;
    Ok(bytes)
}

pub fn require_canonical_json<T: DeserializeOwned + Serialize>(bytes: &[u8]) -> Result<T> {
    let value: T = serde_json::from_slice(bytes)
        .map_err(|_| Error::new("INVALID_JSON", "strict JSON parse failed"))?;
    let canonical = canonical_json(&value)?;
    if canonical != bytes {
        return Err(Error::new(
            "NON_CANONICAL_JSON",
            "input bytes differ from canonical serialization",
        ));
    }
    Ok(value)
}

fn write_value(value: &Value, output: &mut Vec<u8>) -> Result<()> {
    match value {
        Value::Null => Err(Error::new(
            "NULL_NOT_ALLOWED",
            "null is not canonical v1 data",
        )),
        Value::Bool(value) => {
            output.extend_from_slice(if *value { b"true" } else { b"false" });
            Ok(())
        }
        Value::Number(number) => write_number(number, output),
        Value::String(value) => {
            let encoded = serde_json::to_string(value)
                .map_err(|_| Error::new("JSON_SERIALIZATION_FAILED", "string encoding failed"))?;
            output.extend_from_slice(encoded.as_bytes());
            Ok(())
        }
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                write_value(value, output)?;
            }
            output.push(b']');
            Ok(())
        }
        Value::Object(values) => {
            let mut keys: Vec<_> = values.keys().collect();
            keys.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            output.push(b'{');
            for (index, key) in keys.into_iter().enumerate() {
                if !key.is_ascii() {
                    return Err(Error::new(
                        "NON_ASCII_JSON_KEY",
                        "canonical object keys must be ASCII",
                    ));
                }
                if index != 0 {
                    output.push(b',');
                }
                let encoded = serde_json::to_string(key).map_err(|_| {
                    Error::new("JSON_SERIALIZATION_FAILED", "object key encoding failed")
                })?;
                output.extend_from_slice(encoded.as_bytes());
                output.push(b':');
                write_value(&values[key], output)?;
            }
            output.push(b'}');
            Ok(())
        }
    }
}

fn write_number(number: &serde_json::Number, output: &mut Vec<u8>) -> Result<()> {
    let value = if let Some(value) = number.as_i64() {
        value
    } else if let Some(value) = number.as_u64() {
        i64::try_from(value).map_err(|_| {
            Error::new(
                "INTEGER_OUT_OF_RANGE",
                "JSON integer is outside the interoperable signed range",
            )
        })?
    } else {
        return Err(Error::new(
            "NON_INTEGER_JSON_NUMBER",
            "floating-point JSON numbers are forbidden",
        ));
    };
    if !(-JSON_SAFE_INTEGER_MAX..=JSON_SAFE_INTEGER_MAX).contains(&value) {
        return Err(Error::new(
            "INTEGER_OUT_OF_RANGE",
            "JSON integer is outside the interoperable range",
        ));
    }
    output.extend_from_slice(value.to_string().as_bytes());
    Ok(())
}
