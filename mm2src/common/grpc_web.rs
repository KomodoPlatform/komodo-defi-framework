/// The small module implementing gRPC-WEB support
/// Implementation was taken from https://github.com/hyperium/tonic/blob/ddab65ede90f503360b7adb0d7afe6d5b7bb8b02/examples/src/grpc-web/client.rs
/// with minor refactoring
use crate::mm_error::prelude::*;
#[cfg(not(target_arch = "wasm32"))] use crate::wio::slurp_req;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use http::header::{ACCEPT, CONTENT_TYPE};
use prost::DecodeError;

// one byte for the compression flag plus four bytes for the length
const GRPC_HEADER_SIZE: usize = 5;

#[derive(Debug)]
pub enum EncodeBodyError {
    Encode(prost::EncodeError),
}

impl From<prost::EncodeError> for EncodeBodyError {
    fn from(err: prost::EncodeError) -> Self { EncodeBodyError::Encode(err) }
}

fn encode_body<T>(msg: &T) -> Result<Vec<u8>, MmError<EncodeBodyError>>
where
    T: prost::Message,
{
    let mut buf = BytesMut::with_capacity(1024);

    // first skip past the header
    // cannot write it yet since we don't know the size of the
    // encoded message
    buf.put_slice(&[0; 5]);

    // write the message
    msg.encode(&mut buf)?;

    // now we know the size of encoded message and can write the
    // header
    let len = buf.len() - GRPC_HEADER_SIZE;
    {
        let mut buf = &mut buf[..GRPC_HEADER_SIZE];

        // compression flag, 0 means "no compression"
        buf.put_u8(0);

        buf.put_u32(len as u32);
    }

    Ok(buf.split_to(len + GRPC_HEADER_SIZE).freeze().to_vec())
}

#[derive(Debug)]
pub enum DecodeBodyError {
    PayloadTooShort,
    DecodeError(prost::DecodeError),
}

impl From<prost::DecodeError> for DecodeBodyError {
    fn from(err: DecodeError) -> Self { DecodeBodyError::DecodeError(err) }
}

fn decode_body<T>(mut body: Bytes) -> Result<T, MmError<DecodeBodyError>>
where
    T: Default + prost::Message,
{
    if body.len() < GRPC_HEADER_SIZE {
        return MmError::err(DecodeBodyError::PayloadTooShort);
    }
    // ignore the compression flag
    body.advance(1);

    let len = body.get_u32() as usize;
    if body.len() < len {
        return MmError::err(DecodeBodyError::PayloadTooShort);
    }

    let msg = T::decode(&mut body.split_to(len as usize))?;

    Ok(msg)
}

#[derive(Debug)]
pub enum PostGrpcWebErr {
    Http(http::Error),
    Request(String),
    EncodeBody(EncodeBodyError),
    DecodeBody(DecodeBodyError),
}

impl From<EncodeBodyError> for PostGrpcWebErr {
    fn from(err: EncodeBodyError) -> Self { PostGrpcWebErr::EncodeBody(err) }
}

impl From<DecodeBodyError> for PostGrpcWebErr {
    fn from(err: DecodeBodyError) -> Self { PostGrpcWebErr::DecodeBody(err) }
}

impl From<http::Error> for PostGrpcWebErr {
    fn from(err: http::Error) -> Self { PostGrpcWebErr::Http(err) }
}

/// Send POST gRPC WEB HTTPS request and parse response
#[cfg(not(target_arch = "wasm32"))]
pub async fn post_grpc_web<Req, Res>(url: &str, req: &Req) -> Result<Res, MmError<PostGrpcWebErr>>
where
    Req: prost::Message + Send + 'static,
    Res: prost::Message + Default + Send + 'static,
{
    let request = http::Request::builder()
        .version(http::Version::HTTP_11)
        .method(http::Method::POST)
        .uri(url)
        .header(CONTENT_TYPE, "application/grpc-web")
        .header(ACCEPT, "application/grpc-web")
        .body(encode_body(req)?)?;

    let response = slurp_req(request).await.map_to_mm(PostGrpcWebErr::Request)?;

    let reply = decode_body(response.2.into())?;

    Ok(reply)
}
