// Copyright 2017 Parity Technologies (UK) Ltd.
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

use crate::{muxing::{Shutdown, StreamMuxer}, Multiaddr};
use futures::prelude::*;
use std::{fmt, io::{Error as IoError, Read, Write}};
use tokio_io::{AsyncRead, AsyncWrite};

#[derive(Debug, Copy, Clone)]
pub enum EitherError<A, B> { A(A), B(B) }

impl<A, B> fmt::Display for EitherError<A, B>
where
    A: fmt::Display,
    B: fmt::Display
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            EitherError::A(a) => a.fmt(f),
            EitherError::B(b) => b.fmt(f)
        }
    }
}

impl<A, B> std::error::Error for EitherError<A, B>
where
    A: fmt::Debug + std::error::Error,
    B: fmt::Debug + std::error::Error
{
    fn cause(&self) -> Option<&dyn std::error::Error> {
        match self {
            EitherError::A(a) => a.cause(),
            EitherError::B(b) => b.cause()
        }
    }
}

/// Implements `AsyncRead` and `AsyncWrite` and dispatches all method calls to either `A` or `B`.
#[derive(Debug, Copy, Clone)]
pub enum EitherOutput<A, B> { A(A), B(B) }

impl<A, B> AsyncRead for EitherOutput<A, B>
where
    A: AsyncRead,
    B: AsyncRead
{
    #[inline]
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        match self {
            EitherOutput::A(a) => a.prepare_uninitialized_buffer(buf),
            EitherOutput::B(b) => b.prepare_uninitialized_buffer(buf)
        }
    }
}

impl<A, B> Read for EitherOutput<A, B>
where
    A: Read,
    B: Read,
{
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        match self {
            EitherOutput::A(a) => a.read(buf),
            EitherOutput::B(b) => b.read(buf)
        }
    }
}

impl<A, B> AsyncWrite for EitherOutput<A, B>
where
    A: AsyncWrite,
    B: AsyncWrite
{
    #[inline]
    fn shutdown(&mut self) -> Poll<(), IoError> {
        match self {
            EitherOutput::A(a) => a.shutdown(),
            EitherOutput::B(b) => b.shutdown()
        }
    }
}

impl<A, B> Write for EitherOutput<A, B>
where
    A: Write,
    B: Write
{
    #[inline]
    fn write(&mut self, buf: &[u8]) -> Result<usize, IoError> {
        match self {
            EitherOutput::A(a) => a.write(buf),
            EitherOutput::B(b) => b.write(buf)
        }
    }

    #[inline]
    fn flush(&mut self) -> Result<(), IoError> {
        match self {
            EitherOutput::A(a) => a.flush(),
            EitherOutput::B(b) => b.flush()
        }
    }
}

impl<A, B> StreamMuxer for EitherOutput<A, B>
where
    A: StreamMuxer,
    B: StreamMuxer,
{
    type Substream = EitherOutput<A::Substream, B::Substream>;
    type OutboundSubstream = EitherOutbound<A, B>;

    fn poll_inbound(&self) -> Poll<Option<Self::Substream>, IoError> {
        match self {
            EitherOutput::A(inner) => inner.poll_inbound().map(|p| p.map(|o| o.map(EitherOutput::A))),
            EitherOutput::B(inner) => inner.poll_inbound().map(|p| p.map(|o| o.map(EitherOutput::B)))
        }
    }

    fn open_outbound(&self) -> Self::OutboundSubstream {
        match self {
            EitherOutput::A(inner) => EitherOutbound::A(inner.open_outbound()),
            EitherOutput::B(inner) => EitherOutbound::B(inner.open_outbound())
        }
    }

    fn poll_outbound(&self, substream: &mut Self::OutboundSubstream) -> Poll<Option<Self::Substream>, IoError> {
        match (self, substream) {
            (EitherOutput::A(inner), EitherOutbound::A(substream)) =>
                inner.poll_outbound(substream).map(|p| p.map(|o| o.map(EitherOutput::A))),
            (EitherOutput::B(inner), EitherOutbound::B(substream)) =>
                inner.poll_outbound(substream).map(|p| p.map(|o| o.map(EitherOutput::B))),
            _ => panic!("Wrong API usage")
        }
    }

    fn destroy_outbound(&self, substream: Self::OutboundSubstream) {
        match self {
            EitherOutput::A(inner) => match substream {
                EitherOutbound::A(substream) => inner.destroy_outbound(substream),
                _ => panic!("Wrong API usage")
            },
            EitherOutput::B(inner) => match substream {
                EitherOutbound::B(substream) => inner.destroy_outbound(substream),
                _ => panic!("Wrong API usage")
            }
        }
    }

    fn read_substream(&self, sub: &mut Self::Substream, buf: &mut [u8]) -> Poll<usize, IoError> {
        match (self, sub) {
            (EitherOutput::A(inner), EitherOutput::A(sub)) => inner.read_substream(sub, buf),
            (EitherOutput::B(inner), EitherOutput::B(sub)) => inner.read_substream(sub, buf),
            _ => panic!("Wrong API usage")
        }
    }

    fn write_substream(&self, sub: &mut Self::Substream, buf: &[u8]) -> Poll<usize, IoError> {
        match (self, sub) {
            (EitherOutput::A(inner), EitherOutput::A(sub)) => inner.write_substream(sub, buf),
            (EitherOutput::B(inner), EitherOutput::B(sub)) => inner.write_substream(sub, buf),
            _ => panic!("Wrong API usage")
        }
    }

    fn flush_substream(&self, sub: &mut Self::Substream) -> Poll<(), IoError> {
        match (self, sub) {
            (EitherOutput::A(inner), EitherOutput::A(sub)) => inner.flush_substream(sub),
            (EitherOutput::B(inner), EitherOutput::B(sub)) => inner.flush_substream(sub),
            _ => panic!("Wrong API usage")
        }
    }

    fn shutdown_substream(&self, sub: &mut Self::Substream, kind: Shutdown) -> Poll<(), IoError> {
        match (self, sub) {
            (EitherOutput::A(inner), EitherOutput::A(sub)) => inner.shutdown_substream(sub, kind),
            (EitherOutput::B(inner), EitherOutput::B(sub)) => inner.shutdown_substream(sub, kind),
            _ => panic!("Wrong API usage")
        }
    }

    fn destroy_substream(&self, substream: Self::Substream) {
        match self {
            EitherOutput::A(inner) => match substream {
                EitherOutput::A(substream) => inner.destroy_substream(substream),
                _ => panic!("Wrong API usage")
            },
            EitherOutput::B(inner) => match substream {
                EitherOutput::B(substream) => inner.destroy_substream(substream),
                _ => panic!("Wrong API usage")
            }
        }
    }

    fn shutdown(&self, kind: Shutdown) -> Poll<(), IoError> {
        match self {
            EitherOutput::A(inner) => inner.shutdown(kind),
            EitherOutput::B(inner) => inner.shutdown(kind)
        }
    }

    fn flush_all(&self) -> Poll<(), IoError> {
        match self {
            EitherOutput::A(inner) => inner.flush_all(),
            EitherOutput::B(inner) => inner.flush_all()
        }
    }
}

#[derive(Debug, Copy, Clone)]
#[must_use = "futures do nothing unless polled"]
pub enum EitherOutbound<A: StreamMuxer, B: StreamMuxer> {
    A(A::OutboundSubstream),
    B(B::OutboundSubstream)
}

/// Implements `Stream` and dispatches all method calls to either `A` or `B`.
#[derive(Debug, Copy, Clone)]
#[must_use = "futures do nothing unless polled"]
pub enum EitherListenStream<A, B> { A(A), B(B) }

impl<AStream, BStream, AInner, BInner> Stream for EitherListenStream<AStream, BStream>
where
    AStream: Stream<Item = (AInner, Multiaddr), Error = IoError>,
    BStream: Stream<Item = (BInner, Multiaddr), Error = IoError>,
{
    type Item = (EitherFuture<AInner, BInner>, Multiaddr);
    type Error = IoError;

    #[inline]
    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self {
            EitherListenStream::A(a) =>
                a.poll().map(|i| (i.map(|v| (v.map(|(o, addr)| (EitherFuture::A(o), addr)))))),
            EitherListenStream::B(b) =>
                b.poll().map(|i| (i.map(|v| (v.map(|(o, addr)| (EitherFuture::B(o), addr))))))
        }
    }
}

#[derive(Debug, Copy, Clone)]
#[must_use = "futures do nothing unless polled"]
pub enum EitherFuture<A, B> { A(A), B(B) }

impl<AFut, BFut, AItem, BItem, AError, BError> Future for EitherFuture<AFut, BFut>
where
    AFut: Future<Item = AItem, Error = AError>,
    BFut: Future<Item = BItem, Error = BError>
{
    type Item = EitherOutput<AItem, BItem>;
    type Error = EitherError<AError, BError>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self {
            EitherFuture::A(a) =>
                a.poll().map(|v| v.map(EitherOutput::A)).map_err(|e| EitherError::A(e)),
            EitherFuture::B(b) =>
                b.poll().map(|v| v.map(EitherOutput::B)).map_err(|e| EitherError::B(e))
        }
    }
}

