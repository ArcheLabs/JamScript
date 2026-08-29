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

pub fn encode_natural(value: u128) -> Vec<u8> {
    if value <= u64::MAX as u128 {
        encode_natural_u64(value as u64)
    } else {
        let mut out = encode_natural_u64(value as u64);
        out.extend(encode_natural_u64((value >> 64) as u64));
        out
    }
}

fn encode_natural_u64(value: u64) -> Vec<u8> {
    if value < (1 << 7) {
        return vec![value as u8];
    }
    if value < (1 << 56) {
        let extra = ((64 - value.leading_zeros()) as usize - 1) / 7;
        let mut out = vec![(256u16 - (1u16 << (8 - extra))) as u8 | ((value >> (8 * extra)) as u8)];
        out.extend_from_slice(&(value as u64).to_le_bytes()[..extra]);
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
            out.extend(encode_natural(value.len() as u128));
            out.extend(value);
        }
        (TypeIr::String { max }, Value::String(value)) => {
            if value.len() > *max as usize {
                return Err(CodecError::BoundExceeded);
            }
            out.extend(encode_natural(value.len() as u128));
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
            out.extend(encode_natural(values.len() as u128));
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
    fn natural(&mut self) -> Result<u128, CodecError> {
        let first = self.u8()?;
        if first < 0x80 {
            return Ok(first as u128);
        }
        let extra = first.leading_ones() as usize;
        if extra == 8 {
            return Ok(u64::from_le_bytes(
                self.take(8)?
                    .try_into()
                    .map_err(|_| CodecError::InvalidLength)?,
            ) as u128);
        }
        let mut low = [0u8; 8];
        low[..extra].copy_from_slice(self.take(extra)?);
        Ok(u64::from_le_bytes(low) as u128 | (((first & (0x7f >> extra)) as u128) << (8 * extra)))
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
            let len = reader.natural()? as usize;
            if len > *max as usize {
                return Err(CodecError::BoundExceeded);
            }
            Ok(Value::Bytes(reader.take(len)?.to_vec()))
        }
        TypeIr::String { max } => {
            let len = reader.natural()? as usize;
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
            if len > *max as u128 {
                return Err(CodecError::BoundExceeded);
            }
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

    #[test]
    fn natural_and_vec_match_jam_codec() {
        for value in [0u32, 63, 64, 16_383, 16_384, u32::MAX] {
            let ours = encode_natural(value as u128);
            assert_eq!(ours, jam_codec::Compact(value).encode());
            let ty = TypeIr::Bytes { max: u32::MAX };
            assert_eq!(
                encode(&ty, &Value::Bytes(Vec::new())).unwrap(),
                jam_codec::Compact(0u32).encode()
            );
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
}
