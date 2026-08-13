//! Framing for Spotify's in-process Esperanto unary RPC boundary.
//!
//! The desktop bridge sends three big-endian, signed 32-bit length-prefixed
//! byte strings: UTF-8 service name, UTF-8 method name, and protobuf payload.
//! A successful unary response is the raw response protobuf.

use std::future::Future;

use byteorder::{BigEndian, ByteOrder, WriteBytesExt};
use protobuf::Message;

use crate::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnaryCall {
    pub service: String,
    pub method: String,
    pub payload: Vec<u8>,
}

impl UnaryCall {
    pub fn new(service: impl Into<String>, method: impl Into<String>, payload: Vec<u8>) -> Self {
        Self {
            service: service.into(),
            method: method.into(),
            payload,
        }
    }

    pub fn from_message<M: Message>(
        service: impl Into<String>,
        method: impl Into<String>,
        request: &M,
    ) -> Result<Self, Error> {
        Ok(Self::new(service, method, request.write_to_bytes()?))
    }

    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut output =
            Vec::with_capacity(12 + self.service.len() + self.method.len() + self.payload.len());
        write_part(&mut output, self.service.as_bytes())?;
        write_part(&mut output, self.method.as_bytes())?;
        write_part(&mut output, &self.payload)?;
        Ok(output)
    }

    pub fn decode(input: &[u8]) -> Result<Self, Error> {
        let mut offset = 0;
        let service = read_part(input, &mut offset)?;
        let method = read_part(input, &mut offset)?;
        let payload = read_part(input, &mut offset)?;
        if offset != input.len() {
            return Err(Error::failed_precondition(
                "Esperanto unary frame contains trailing bytes",
            ));
        }

        Ok(Self {
            service: String::from_utf8(service)?,
            method: String::from_utf8(method)?,
            payload,
        })
    }
}

/// Encode a typed request, execute the framed unary call, and decode the raw
/// protobuf response. The executor deliberately remains transport-agnostic.
pub async fn call_unary<Request, Response, Execute, ExecuteFuture>(
    service: &str,
    method: &str,
    request: &Request,
    execute: Execute,
) -> Result<Response, Error>
where
    Request: Message,
    Response: Message,
    Execute: FnOnce(Vec<u8>) -> ExecuteFuture,
    ExecuteFuture: Future<Output = Result<Vec<u8>, Error>>,
{
    let frame = UnaryCall::from_message(service, method, request)?.encode()?;
    let response = execute(frame).await?;
    Response::parse_from_bytes(&response).map_err(Error::failed_precondition)
}

fn write_part(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), Error> {
    let length = i32::try_from(bytes.len())
        .map_err(|_| Error::out_of_range("Esperanto unary frame part exceeds i32::MAX"))?;
    output.write_i32::<BigEndian>(length)?;
    output.extend_from_slice(bytes);
    Ok(())
}

fn read_part(input: &[u8], offset: &mut usize) -> Result<Vec<u8>, Error> {
    let length_end = offset
        .checked_add(4)
        .filter(|end| *end <= input.len())
        .ok_or_else(|| Error::failed_precondition("truncated Esperanto unary frame length"))?;
    let length = BigEndian::read_i32(&input[*offset..length_end]);
    let length = usize::try_from(length)
        .map_err(|_| Error::failed_precondition("negative Esperanto unary frame length"))?;
    let value_end = length_end
        .checked_add(length)
        .filter(|end| *end <= input.len())
        .ok_or_else(|| Error::failed_precondition("truncated Esperanto unary frame value"))?;
    *offset = value_end;
    Ok(input[length_end..value_end].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_unary_frame_is_big_endian_length_prefixed() {
        let call = UnaryCall::new("svc", "Get", vec![0xaa, 0xbb]);
        let encoded = call.encode().unwrap();
        assert_eq!(
            encoded,
            vec![
                0, 0, 0, 3, b's', b'v', b'c', 0, 0, 0, 3, b'G', b'e', b't', 0, 0, 0, 2, 0xaa, 0xbb,
            ]
        );
        assert_eq!(UnaryCall::decode(&encoded).unwrap(), call);
    }

    #[test]
    fn malformed_unary_frames_are_rejected() {
        assert!(UnaryCall::decode(&[0, 0, 0]).is_err());
        assert!(UnaryCall::decode(&[0xff, 0xff, 0xff, 0xff]).is_err());

        let mut trailing = UnaryCall::new("s", "m", Vec::new()).encode().unwrap();
        trailing.push(0);
        assert!(UnaryCall::decode(&trailing).is_err());
    }
}
