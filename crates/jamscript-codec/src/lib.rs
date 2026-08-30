//! The single dynamic codec used by JamScript ABI surfaces.
//!
//! This module intentionally mirrors `jam-codec 0.1.1`: fixed-width
//! primitives are little-endian and sequence/string lengths use JAM's
//! general-natural encoding (`jam_codec::Compact`).  It does not define a
//! second wire format for actions or state.

use jamscript_ir::TypeIr;
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Unit,
    Bool(bool),
    Unsigned(u128),
    Signed(i128),
    Bytes(Vec<u8>),
    String(String),
    Array(Vec<Value>),
    Tuple(Vec<Value>),
    Record(Vec<(String, Value)>),
    Option(Option<Box<Value>>),
    Enum { index: u8, value: Box<Value> },
    Result(Result<Box<Value>, Box<Value>>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodecError {
    TypeMismatch(&'static str),
    UnexpectedEof,
    TrailingBytes,
    InvalidBool,
    InvalidUtf8,
    InvalidVariant,
    InvalidLength,
    OutOfRange,
    BoundExceeded,
    UnsupportedType,
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for CodecError {}

pub fn encode(ty: &TypeIr, value: &Value) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    encode_into(ty, value, &mut out)?;
    Ok(out)
}

pub fn decode(ty: &TypeIr, bytes: &[u8]) -> Result<Value, CodecError> {
    let mut reader = Reader { bytes, offset: 0 };
    let value = decode_from(ty, &mut reader)?;
    if reader.offset != bytes.len() {
        return Err(CodecError::TrailingBytes);
    }
    Ok(value)
}

pub fn max_encoded_len(ty: &TypeIr) -> Result<usize, CodecError> {
    ty.max_encoded_len()
        .map_err(|_| CodecError::UnsupportedType)
}

pub fn encode_natural(value: u64) -> Vec<u8> {
    encode_natural_u64(value)
}

fn encode_natural_u64(value: u64) -> Vec<u8> {
    if value < (1 << 7) {
        return vec![value as u8];
    }
    if value < (1 << 56) {
        let extra = ((64 - value.leading_zeros()) as usize - 1) / 7;
        let mut out = vec![(256u16 - (1u16 << (8 - extra))) as u8 | ((value >> (8 * extra)) as u8)];
        out.extend_from_slice(&value.to_le_bytes()[..extra]);
        return out;
    }
    let mut out = vec![0xff];
    out.extend_from_slice(&value.to_le_bytes());
    out
}

fn encode_into(ty: &TypeIr, value: &Value, out: &mut Vec<u8>) -> Result<(), CodecError> {
    match (ty, value) {
        (TypeIr::Unit, Value::Unit) => {}
        (TypeIr::Bool, Value::Bool(value)) => out.push(u8::from(*value)),
        (TypeIr::U8, Value::Unsigned(value)) => push_unsigned(out, *value, u8::MAX as u128, 1)?,
        (TypeIr::U16, Value::Unsigned(value)) => push_unsigned(out, *value, u16::MAX as u128, 2)?,
        (TypeIr::U32, Value::Unsigned(value)) => push_unsigned(out, *value, u32::MAX as u128, 4)?,
        (TypeIr::U64, Value::Unsigned(value)) => push_unsigned(out, *value, u64::MAX as u128, 8)?,
        (TypeIr::U128, Value::Unsigned(value)) => push_unsigned(out, *value, u128::MAX, 16)?,
        (TypeIr::I8, Value::Signed(value)) => {
            push_signed(out, *value, i8::MIN as i128, i8::MAX as i128, 1)?
        }
        (TypeIr::I16, Value::Signed(value)) => {
            push_signed(out, *value, i16::MIN as i128, i16::MAX as i128, 2)?
        }
        (TypeIr::I32, Value::Signed(value)) => {
            push_signed(out, *value, i32::MIN as i128, i32::MAX as i128, 4)?
        }
        (TypeIr::I64, Value::Signed(value)) => {
            push_signed(out, *value, i64::MIN as i128, i64::MAX as i128, 8)?
        }
        (TypeIr::I128, Value::Signed(value)) => push_signed(out, *value, i128::MIN, i128::MAX, 16)?,
        (TypeIr::Address, Value::Bytes(value)) if value.len() == 32 => out.extend(value),
        (TypeIr::FixedBytes { len }, Value::Bytes(value)) if value.len() == *len as usize => {
            out.extend(value)
        }
        (TypeIr::Bytes { max }, Value::Bytes(value)) => {
            if value.len() > *max as usize {
                return Err(CodecError::BoundExceeded);
            }
            out.extend(encode_natural(
                u64::try_from(value.len()).map_err(|_| CodecError::OutOfRange)?,
            ));
            out.extend(value);
        }
        (TypeIr::String { max }, Value::String(value)) => {
            if value.len() > *max as usize {
                return Err(CodecError::BoundExceeded);
            }
            out.extend(encode_natural(
                u64::try_from(value.len()).map_err(|_| CodecError::OutOfRange)?,
            ));
            out.extend(value.as_bytes());
        }
        (TypeIr::FixedArray { item, len }, Value::Array(values))
            if values.len() == *len as usize =>
        {
            for value in values {
                encode_into(item, value, out)?;
            }
        }
        (TypeIr::Array { item, max }, Value::Array(values)) => {
            if values.len() > *max as usize {
                return Err(CodecError::BoundExceeded);
            }
            out.extend(encode_natural(
                u64::try_from(values.len()).map_err(|_| CodecError::OutOfRange)?,
            ));
            for value in values {
                encode_into(item, value, out)?;
            }
        }
        (TypeIr::Option { item }, Value::Option(value)) => match value {
            None => out.push(0),
            Some(value) => {
                out.push(1);
                encode_into(item, value, out)?;
            }
        },
        (TypeIr::Tuple { items }, Value::Tuple(values)) if items.len() == values.len() => {
            for (item, value) in items.iter().zip(values) {
                encode_into(item, value, out)?;
            }
        }
        (TypeIr::Record { fields }, Value::Record(values)) if fields.len() == values.len() => {
            for (field, (name, value)) in fields.iter().zip(values) {
                if field.name != *name {
                    return Err(CodecError::TypeMismatch("record field order"));
                }
                encode_into(&field.ty, value, out)?;
            }
        }
        (TypeIr::Enum { variants }, Value::Enum { index, value }) => {
            let variant = variants
                .iter()
                .find(|variant| variant.index == *index)
                .ok_or(CodecError::InvalidVariant)?;
            out.push(*index);
            encode_into(&variant.ty, value, out)?;
        }
        (TypeIr::Result { ok, err }, Value::Result(value)) => match value {
            Ok(value) => {
                out.push(0);
                encode_into(ok, value, out)?;
            }
            Err(value) => {
                out.push(1);
                encode_into(err, value, out)?;
            }
        },
        (TypeIr::Unsupported(_), _) => return Err(CodecError::UnsupportedType),
        (TypeIr::Address, Value::Bytes(_)) | (TypeIr::FixedBytes { .. }, Value::Bytes(_)) => {
            return Err(CodecError::InvalidLength)
        }
        _ => return Err(CodecError::TypeMismatch("value")),
    }
    Ok(())
}

fn push_unsigned(
    out: &mut Vec<u8>,
    value: u128,
    max: u128,
    width: usize,
) -> Result<(), CodecError> {
    if value > max {
        return Err(CodecError::OutOfRange);
    }
    out.extend_from_slice(&value.to_le_bytes()[..width]);
    Ok(())
}

fn push_signed(
    out: &mut Vec<u8>,
    value: i128,
    min: i128,
    max: i128,
    width: usize,
) -> Result<(), CodecError> {
    if value < min || value > max {
        return Err(CodecError::OutOfRange);
    }
    out.extend_from_slice(&value.to_le_bytes()[..width]);
    Ok(())
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], CodecError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(CodecError::InvalidLength)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(CodecError::UnexpectedEof)?;
        self.offset = end;
        Ok(value)
    }
    fn u8(&mut self) -> Result<u8, CodecError> {
        Ok(self.take(1)?[0])
    }
    fn natural(&mut self) -> Result<u64, CodecError> {
        let first = self.u8()?;
        if first < 0x80 {
            return Ok(first as u64);
        }
        let extra = first.leading_ones() as usize;
        if extra == 8 {
            return Ok(u64::from_le_bytes(
                self.take(8)?
                    .try_into()
                    .map_err(|_| CodecError::InvalidLength)?,
            ));
        }
        let mut low = [0u8; 8];
        low[..extra].copy_from_slice(self.take(extra)?);
        Ok(u64::from_le_bytes(low) | (((first & (0x7f >> extra)) as u64) << (8 * extra)))
    }
}

fn decode_from(ty: &TypeIr, reader: &mut Reader<'_>) -> Result<Value, CodecError> {
    match ty {
        TypeIr::Unit => Ok(Value::Unit),
        TypeIr::Bool => match reader.u8()? {
            0 => Ok(Value::Bool(false)),
            1 => Ok(Value::Bool(true)),
            _ => Err(CodecError::InvalidBool),
        },
        TypeIr::U8 => Ok(Value::Unsigned(reader.u8()? as u128)),
        TypeIr::U16 => Ok(Value::Unsigned(
            u16::from_le_bytes(reader.take(2)?.try_into().unwrap()) as u128,
        )),
        TypeIr::U32 => Ok(Value::Unsigned(
            u32::from_le_bytes(reader.take(4)?.try_into().unwrap()) as u128,
        )),
        TypeIr::U64 => Ok(Value::Unsigned(
            u64::from_le_bytes(reader.take(8)?.try_into().unwrap()) as u128,
        )),
        TypeIr::U128 => Ok(Value::Unsigned(u128::from_le_bytes(
            reader.take(16)?.try_into().unwrap(),
        ))),
        TypeIr::I8 => Ok(Value::Signed(i8::from_le_bytes([reader.u8()?]) as i128)),
        TypeIr::I16 => Ok(Value::Signed(
            i16::from_le_bytes(reader.take(2)?.try_into().unwrap()) as i128,
        )),
        TypeIr::I32 => Ok(Value::Signed(
            i32::from_le_bytes(reader.take(4)?.try_into().unwrap()) as i128,
        )),
        TypeIr::I64 => Ok(Value::Signed(
            i64::from_le_bytes(reader.take(8)?.try_into().unwrap()) as i128,
        )),
        TypeIr::I128 => Ok(Value::Signed(i128::from_le_bytes(
            reader.take(16)?.try_into().unwrap(),
        ))),
        TypeIr::Address => Ok(Value::Bytes(reader.take(32)?.to_vec())),
        TypeIr::FixedBytes { len } => Ok(Value::Bytes(reader.take(*len as usize)?.to_vec())),
        TypeIr::Bytes { max } => {
            let len = usize::try_from(reader.natural()?).map_err(|_| CodecError::InvalidLength)?;
            if len > *max as usize {
                return Err(CodecError::BoundExceeded);
            }
            Ok(Value::Bytes(reader.take(len)?.to_vec()))
        }
        TypeIr::String { max } => {
            let len = usize::try_from(reader.natural()?).map_err(|_| CodecError::InvalidLength)?;
            if len > *max as usize {
                return Err(CodecError::BoundExceeded);
            }
            Ok(Value::String(
                String::from_utf8(reader.take(len)?.to_vec())
                    .map_err(|_| CodecError::InvalidUtf8)?,
            ))
        }
        TypeIr::FixedArray { item, len } => Ok(Value::Array(
            (0..*len)
                .map(|_| decode_from(item, reader))
                .collect::<Result<_, _>>()?,
        )),
        TypeIr::Array { item, max } => {
            let len = reader.natural()?;
            if len > *max as u64 {
                return Err(CodecError::BoundExceeded);
            }
            let len = usize::try_from(len).map_err(|_| CodecError::InvalidLength)?;
            Ok(Value::Array(
                (0..len)
                    .map(|_| decode_from(item, reader))
                    .collect::<Result<_, _>>()?,
            ))
        }
        TypeIr::Option { item } => match reader.u8()? {
            0 => Ok(Value::Option(None)),
            1 => Ok(Value::Option(Some(Box::new(decode_from(item, reader)?)))),
            _ => Err(CodecError::InvalidVariant),
        },
        TypeIr::Tuple { items } => Ok(Value::Tuple(
            items
                .iter()
                .map(|item| decode_from(item, reader))
                .collect::<Result<_, _>>()?,
        )),
        TypeIr::Record { fields } => Ok(Value::Record(
            fields
                .iter()
                .map(|field| Ok((field.name.clone(), decode_from(&field.ty, reader)?)))
                .collect::<Result<_, CodecError>>()?,
        )),
        TypeIr::Enum { variants } => {
            let index = reader.u8()?;
            let variant = variants
                .iter()
                .find(|variant| variant.index == index)
                .ok_or(CodecError::InvalidVariant)?;
            Ok(Value::Enum {
                index,
                value: Box::new(decode_from(&variant.ty, reader)?),
            })
        }
        TypeIr::Result { ok, err } => match reader.u8()? {
            0 => Ok(Value::Result(Ok(Box::new(decode_from(ok, reader)?)))),
            1 => Ok(Value::Result(Err(Box::new(decode_from(err, reader)?)))),
            _ => Err(CodecError::InvalidVariant),
        },
        TypeIr::Unsupported(_) => Err(CodecError::UnsupportedType),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jam_codec::Encode;
    use jamscript_ir::{FieldIr, TypeIr};
    use serde_json::Value as JsonValue;

    #[test]
    fn natural_and_vec_match_jam_codec() {
        let values = [
            0,
            1,
            127,
            128,
            255,
            256,
            (1u64 << 14) - 1,
            1u64 << 14,
            (1u64 << 21) - 1,
            1u64 << 21,
            (1u64 << 28) - 1,
            1u64 << 28,
            (1u64 << 35) - 1,
            1u64 << 35,
            (1u64 << 42) - 1,
            1u64 << 42,
            (1u64 << 49) - 1,
            1u64 << 49,
            (1u64 << 56) - 1,
            1u64 << 56,
            u64::MAX,
        ];
        for value in values {
            assert_eq!(encode_natural(value), jam_codec::Compact(value).encode());
        }
    }

    #[test]
    fn sequence_boundaries_round_trip_with_u64_natural() {
        for len in [0usize, 1, 127, 128, 255, 256, 16_383, 16_384] {
            let prefix = jam_codec::Compact(len as u64).encode();
            let bytes_type = TypeIr::Bytes { max: 16_384 };
            let bytes_value = Value::Bytes(vec![0xaa; len]);
            let encoded = encode(&bytes_type, &bytes_value).unwrap();
            assert_eq!(&encoded[..prefix.len()], prefix.as_slice());
            assert_eq!(decode(&bytes_type, &encoded).unwrap(), bytes_value);

            let string_type = TypeIr::String { max: 16_384 };
            let string_value = Value::String("a".repeat(len));
            let encoded = encode(&string_type, &string_value).unwrap();
            assert_eq!(&encoded[..prefix.len()], prefix.as_slice());
            assert_eq!(decode(&string_type, &encoded).unwrap(), string_value);

            let array_type = TypeIr::Array {
                item: Box::new(TypeIr::U8),
                max: 16_384,
            };
            let array_value = Value::Array(vec![Value::Unsigned(0); len]);
            let encoded = encode(&array_type, &array_value).unwrap();
            assert_eq!(&encoded[..prefix.len()], prefix.as_slice());
            assert_eq!(decode(&array_type, &encoded).unwrap(), array_value);
        }
    }

    #[test]
    fn nested_record_round_trips() {
        let ty = TypeIr::Record {
            fields: vec![
                FieldIr {
                    name: "ok".into(),
                    ty: TypeIr::Bool,
                },
                FieldIr {
                    name: "xs".into(),
                    ty: TypeIr::Array {
                        item: Box::new(TypeIr::U64),
                        max: 4,
                    },
                },
            ],
        };
        let value = Value::Record(vec![
            ("ok".into(), Value::Bool(true)),
            ("xs".into(), Value::Array(vec![Value::Unsigned(7)])),
        ]);
        assert_eq!(decode(&ty, &encode(&ty, &value).unwrap()).unwrap(), value);
    }

    fn type_from_json(value: &JsonValue) -> TypeIr {
        let object = value.as_object().unwrap();
        let kind = object.get("kind").unwrap().as_str().unwrap();
        match kind {
            "unit" => TypeIr::Unit,
            "bool" => TypeIr::Bool,
            "u8" => TypeIr::U8,
            "u16" => TypeIr::U16,
            "u32" => TypeIr::U32,
            "u64" => TypeIr::U64,
            "u128" => TypeIr::U128,
            "i8" => TypeIr::I8,
            "i16" => TypeIr::I16,
            "i32" => TypeIr::I32,
            "i64" => TypeIr::I64,
            "i128" => TypeIr::I128,
            "address" => TypeIr::Address,
            "fixedBytes" => TypeIr::FixedBytes {
                len: object["len"].as_u64().unwrap() as u32,
            },
            "bytes" => TypeIr::Bytes {
                max: object["max"].as_u64().unwrap() as u32,
            },
            "string" => TypeIr::String {
                max: object["max"].as_u64().unwrap() as u32,
            },
            "fixedArray" => TypeIr::FixedArray {
                item: Box::new(type_from_json(&object["item"])),
                len: object["len"].as_u64().unwrap() as u32,
            },
            "array" => TypeIr::Array {
                item: Box::new(type_from_json(&object["item"])),
                max: object["max"].as_u64().unwrap() as u32,
            },
            "option" => TypeIr::Option {
                item: Box::new(type_from_json(&object["item"])),
            },
            "tuple" => TypeIr::Tuple {
                items: object["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(type_from_json)
                    .collect(),
            },
            "record" => TypeIr::Record {
                fields: object["fields"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|field| FieldIr {
                        name: field["name"].as_str().unwrap().into(),
                        ty: type_from_json(&field["type"]),
                    })
                    .collect(),
            },
            "enum" => TypeIr::Enum {
                variants: object["variants"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|variant| jamscript_ir::VariantIr {
                        name: variant["name"].as_str().unwrap().into(),
                        index: variant["index"].as_u64().unwrap() as u8,
                        ty: type_from_json(&variant["type"]),
                    })
                    .collect(),
            },
            "result" => TypeIr::Result {
                ok: Box::new(type_from_json(&object["ok"])),
                err: Box::new(type_from_json(&object["err"])),
            },
            _ => panic!("unknown vector type {kind}"),
        }
    }

    fn value_from_json(ty: &TypeIr, value: &JsonValue) -> Value {
        match ty {
            TypeIr::Unit => Value::Unit,
            TypeIr::Bool => Value::Bool(value.as_bool().unwrap()),
            TypeIr::U8 | TypeIr::U16 | TypeIr::U32 | TypeIr::U64 | TypeIr::U128 => Value::Unsigned(
                value
                    .as_u64()
                    .map(u128::from)
                    .unwrap_or_else(|| value.as_str().unwrap().parse().unwrap()),
            ),
            TypeIr::I8 | TypeIr::I16 | TypeIr::I32 | TypeIr::I64 | TypeIr::I128 => Value::Signed(
                value
                    .as_i64()
                    .map(i128::from)
                    .unwrap_or_else(|| value.as_str().unwrap().parse().unwrap()),
            ),
            TypeIr::Address | TypeIr::FixedBytes { .. } | TypeIr::Bytes { .. } => {
                Value::Bytes(hex::decode(value.as_str().unwrap()).unwrap())
            }
            TypeIr::String { .. } => Value::String(value.as_str().unwrap().into()),
            TypeIr::FixedArray { item, .. } | TypeIr::Array { item, .. } => Value::Array(
                value
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value_from_json(item, value))
                    .collect(),
            ),
            TypeIr::Option { item } => match value {
                JsonValue::Null => Value::Option(None),
                value => Value::Option(Some(Box::new(value_from_json(item, value)))),
            },
            TypeIr::Tuple { items } => Value::Tuple(
                items
                    .iter()
                    .zip(value.as_array().unwrap())
                    .map(|(ty, value)| value_from_json(ty, value))
                    .collect(),
            ),
            TypeIr::Record { fields } => Value::Record(
                fields
                    .iter()
                    .map(|field| {
                        (
                            field.name.clone(),
                            value_from_json(&field.ty, &value[&field.name]),
                        )
                    })
                    .collect(),
            ),
            TypeIr::Enum { variants } => {
                let index = value["index"].as_u64().unwrap() as u8;
                let variant = variants
                    .iter()
                    .find(|variant| variant.index == index)
                    .unwrap();
                Value::Enum {
                    index,
                    value: Box::new(value_from_json(&variant.ty, &value["value"])),
                }
            }
            TypeIr::Result { ok, err } => {
                if let Some(value) = value.get("ok") {
                    Value::Result(Ok(Box::new(value_from_json(ok, value))))
                } else {
                    Value::Result(Err(Box::new(value_from_json(err, &value["err"]))))
                }
            }
            TypeIr::Unsupported(_) => panic!("unsupported vector type"),
        }
    }

    #[test]
    fn shared_vectors_are_normative_codec_inputs() {
        for source in [
            include_str!("../../../test-vectors/abi-codec/primitives.json"),
            include_str!("../../../test-vectors/abi-codec/composites.json"),
        ] {
            for vector in serde_json::from_str::<Vec<JsonValue>>(source).unwrap() {
                let ty = type_from_json(&vector["type"]);
                let value = value_from_json(&ty, &vector["value"]);
                let expected = hex::decode(vector["encodedHex"].as_str().unwrap()).unwrap();
                assert_eq!(encode(&ty, &value).unwrap(), expected);
                assert_eq!(decode(&ty, &expected).unwrap(), value);
            }
        }
    }

    #[test]
    fn malformed_values_are_rejected() {
        assert_eq!(decode(&TypeIr::Bool, &[2]), Err(CodecError::InvalidBool));
        assert_eq!(
            decode(
                &TypeIr::Option {
                    item: Box::new(TypeIr::U8)
                },
                &[2]
            ),
            Err(CodecError::InvalidVariant)
        );
        assert_eq!(
            decode(
                &TypeIr::Enum {
                    variants: vec![jamscript_ir::VariantIr {
                        name: "Only".into(),
                        index: 0,
                        ty: TypeIr::Unit
                    }]
                },
                &[1]
            ),
            Err(CodecError::InvalidVariant)
        );
        assert_eq!(
            decode(
                &TypeIr::Result {
                    ok: Box::new(TypeIr::Unit),
                    err: Box::new(TypeIr::Unit)
                },
                &[2]
            ),
            Err(CodecError::InvalidVariant)
        );
        assert_eq!(decode(&TypeIr::U32, &[0]), Err(CodecError::UnexpectedEof));
        assert_eq!(
            decode(&TypeIr::String { max: 4 }, &[1, 0xff]),
            Err(CodecError::InvalidUtf8)
        );
        assert_eq!(decode(&TypeIr::U8, &[0, 1]), Err(CodecError::TrailingBytes));
        assert_eq!(
            encode(&TypeIr::Bytes { max: 1 }, &Value::Bytes(vec![1, 2])),
            Err(CodecError::BoundExceeded)
        );
        assert_eq!(
            encode(&TypeIr::FixedBytes { len: 2 }, &Value::Bytes(vec![1])),
            Err(CodecError::InvalidLength)
        );
        assert_eq!(
            encode(&TypeIr::Address, &Value::Bytes(vec![0; 31])),
            Err(CodecError::InvalidLength)
        );
    }
}
