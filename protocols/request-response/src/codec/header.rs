// Copyright 2020 Parity Technologies (UK) Ltd.
//
// Permission is hereby granted, free of charge, to any person obtaining a
// copy of this software and associated documentation files (the "Software"),
// to deal in the Software without restriction, including without limitation
// the rights to use, copy, modify, merge, publish, distribute, sublicense,
// and/or sell copies of the Software, and to permit persons to whom the
// Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
// OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
// FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use futures::prelude::*;
use libp2p_core::ProtocolName;
use minicbor::{Encode, Decode};
use std::io;
use super::RequestResponseCodec;
use unsigned_varint::{aio, io::ReadError};

/// A protocol request header.
//
// Currently empty but added for future extensibility.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct RequestHeader;

/// A protocol response header.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct ResponseHeader {
    /// The number of additional requests the receiver is willing to process.
    #[n(0)] pub credit: u16
}

/// A protocol request consisting of header and payload data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request<B> {
    header: RequestHeader,
    body: B
}

/// A protocol response consisting of header and payload data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response<B> {
    header: ResponseHeader,
    body: B
}

impl<B> Request<B> {
    /// Create a new request with the given payload.
    pub fn new(body: B) -> Self {
        Request { header: RequestHeader, body }
    }

    /// Access the request header.
    pub fn header(&self) -> &RequestHeader {
        &self.header
    }

    /// Access the request body.
    pub fn body(&self) -> &B {
        &self.body
    }

    /// Consume this request and return header and body.
    pub fn into_parts(self) -> (RequestHeader, B) {
        (self.header, self.body)
    }
}

impl<B> Response<B> {
    /// Create a new response with the given payload.
    pub fn new(body: B) -> Self {
        Response {
            header: ResponseHeader { credit: 0 },
            body
        }
    }

    /// Access the response header.
    pub fn header(&self) -> &ResponseHeader {
        &self.header
    }

    /// Access the response body.
    pub fn body(&self) -> &B {
        &self.body
    }

    /// Specify the number of requests the sender is allowed to send.
    pub fn set_credit(&mut self, credit: u16) -> &mut Self {
        self.header.credit = credit;
        self
    }

    /// Consume this response and return header and body.
    pub fn into_parts(self) -> (ResponseHeader, B) {
        (self.header, self.body)
    }
}

/// A wrapper around a `ProtocolName` impl which augments the protocol name.
///
/// The type implements `ProtocolName` itself and creates a name for a
/// request-response protocol based on the protocol name of the wrapped type.
#[derive(Debug, Clone)]
pub struct ProtocolWrapper<P>(P, Bytes);

impl<P: ProtocolName> ProtocolWrapper<P> {
    pub fn v1(p: P) -> Self {
        let mut full = BytesMut::from(b"/request-response/header/1".as_ref());
        full.extend_from_slice(p.protocol_name());
        ProtocolWrapper(p, full.freeze())
    }
}

impl<P> ProtocolName for ProtocolWrapper<P> {
    fn protocol_name(&self) -> &[u8] {
        self.1.as_ref()
    }
}

/// A `RequestResponseCodec` wrapper that adds headers to the payload data.
#[derive(Debug, Clone)]
pub struct Codec<C> {
    /// The wrapped codec.
    inner: C,
    /// Encoding/decoding buffer.
    buffer: Vec<u8>
}

impl<C> Codec<C> {
    /// Create a codec by wrapping an existing one.
    pub fn new(c: C) -> Self {
        Codec { inner: c, buffer: Vec::new() }
    }

    /// Read and decode a request header.
    async fn read_header<T, H>(&mut self, io: &mut T) -> io::Result<H>
    where
        T: AsyncRead + Unpin + Send,
        H: for<'a> minicbor::Decode<'a>
    {
        let header_len = aio::read_u32(&mut *io).await
            .map_err(|e| match e {
                ReadError::Io(e) => e,
                other => io::Error::new(io::ErrorKind::Other, other)
            })?;
        self.buffer.resize(u32_to_usize(header_len), 0u8);
        io.read_exact(&mut self.buffer).await?;
        minicbor::decode(&self.buffer).map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }

    /// Encode and write a response header.
    async fn write_header<T, H>(&mut self, hdr: &H, io: &mut T) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
        H: minicbor::Encode
    {
        self.buffer.clear();
        minicbor::encode(hdr, &mut self.buffer).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let mut b = unsigned_varint::encode::u32_buffer();
        assert!(self.buffer.len() < u32_to_usize(u32::MAX));
        let header_len = unsigned_varint::encode::u32(self.buffer.len() as u32, &mut b);
        io.write_all(header_len).await?;
        io.write_all(&self.buffer).await?;
        Ok(())
    }
}

#[async_trait]
impl<C> RequestResponseCodec for Codec<C>
where
    C: RequestResponseCodec + Send,
    C::Protocol: Sync
{
    type Protocol = ProtocolWrapper<C::Protocol>;
    type Request = Request<C::Request>;
    type Response = Response<C::Response>;

    async fn read_request<T>(&mut self, p: &Self::Protocol, io: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send
    {
        let header = self.read_header(io).await?;
        let body = self.inner.read_request(&p.0, io).await?;
        Ok(Request { header, body })
    }

    async fn read_response<T>(&mut self, p: &Self::Protocol, io: &mut T) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send
    {
        let header = self.read_header(io).await?;
        let body = self.inner.read_response(&p.0, io).await?;
        Ok(Response { header, body })
    }

    async fn write_request<T>(&mut self, p: &Self::Protocol, io: &mut T, r: Self::Request) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send
    {
        self.write_header(&r.header, io).await?;
        self.inner.write_request(&p.0, io, r.body).await
    }

    async fn write_response<T>(&mut self, p: &Self::Protocol, io: &mut T, r: Self::Response) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send
    {
        self.write_header(&r.header, io).await?;
        self.inner.write_response(&p.0, io, r.body).await
    }
}

#[cfg(any(target_pointer_width = "64", target_pointer_width = "32"))]
fn u32_to_usize(n: u32) -> usize {
    n as usize
}
