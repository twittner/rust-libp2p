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

//! Contains the `Dialer` wrapper, which allows raw communications with a listener.

use bytes::Bytes;
use crate::protocol::DialerToListenerMessage;
use crate::protocol::ListenerToDialerMessage;
use crate::protocol::MultistreamSelectError;
use crate::protocol::{MULTISTREAM_PROTOCOL, MULTISTREAM_PROTOCOL_WITH_LF};
use crate::protocol::RawSlice;
use futures::{prelude::*, sink, Async, AsyncSink, StartSend, try_ready};
use tokio_codec::Framed;
use tokio_io::{AsyncRead, AsyncWrite};
use unsigned_varint::{decode, codec::UviBytes};

/// Wraps around a `AsyncRead+AsyncWrite`.
/// Assumes that we're on the dialer's side. Produces and accepts messages.
pub struct Dialer<R, N> {
    inner: Framed<R, UviBytes<RawSlice>>,
    handshake_finished: bool,
    _mark: std::marker::PhantomData<N>
}

impl<R, N> Dialer<R, N>
where
    R: AsyncRead + AsyncWrite
{
    pub fn new(inner: R) -> DialerFuture<R, N> {
        let mut codec = UviBytes::default();
        codec.set_max_len(std::u16::MAX as usize);
        codec.set_suffix(b"\n");
        let sender = Framed::new(inner, codec);
        DialerFuture {
            inner: sender.send(MULTISTREAM_PROTOCOL.into()),
            _mark: std::marker::PhantomData
        }
    }

    /// Grants back the socket. Typically used after a `ProtocolAck` has been received.
    #[inline]
    pub fn into_inner(self) -> R {
        self.inner.into_inner()
    }
}

impl<R, N> Sink for Dialer<R, N>
where
    R: AsyncRead + AsyncWrite,
    N: AsRef<[u8]>
{
    type SinkItem = DialerToListenerMessage<N>;
    type SinkError = MultistreamSelectError;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        match item {
            DialerToListenerMessage::ProtocolRequest { name } => {
                if !name.as_ref().starts_with(b"/") {
                    return Err(MultistreamSelectError::WrongProtocolName);
                }
                match self.inner.start_send(name.as_ref().into())? {
                    AsyncSink::Ready => Ok(AsyncSink::Ready),
                    AsyncSink::NotReady(_) => {
                        let item = DialerToListenerMessage::ProtocolRequest { name };
                        Ok(AsyncSink::NotReady(item))
                    }
                }
            }
            DialerToListenerMessage::ProtocolsListRequest => {
                match self.inner.start_send(b"ls"[..].into())? {
                    AsyncSink::Ready => Ok(AsyncSink::Ready),
                    AsyncSink::NotReady(_) => {
                        let item = DialerToListenerMessage::ProtocolsListRequest;
                        Ok(AsyncSink::NotReady(item))
                    }
                }
            }
        }
    }

    #[inline]
    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        Ok(self.inner.poll_complete()?)
    }

    #[inline]
    fn close(&mut self) -> Poll<(), Self::SinkError> {
        Ok(self.inner.close()?)
    }
}

impl<R, N> Stream for Dialer<R, N>
where
    R: AsyncRead + AsyncWrite
{
    type Item = ListenerToDialerMessage<Bytes>;
    type Error = MultistreamSelectError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        loop {
            let mut frame = match self.inner.poll() {
                Ok(Async::Ready(Some(frame))) => frame,
                Ok(Async::Ready(None)) => return Ok(Async::Ready(None)),
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                Err(err) => return Err(err.into()),
            };

            if !self.handshake_finished {
                if frame == MULTISTREAM_PROTOCOL_WITH_LF {
                    self.handshake_finished = true;
                    continue;
                } else {
                    return Err(MultistreamSelectError::FailedHandshake);
                }
            }

            if frame.get(0) == Some(&b'/') && frame.last() == Some(&b'\n') {
                let frame_len = frame.len();
                let protocol = frame.split_to(frame_len - 1);
                return Ok(Async::Ready(Some(ListenerToDialerMessage::ProtocolAck {
                    name: protocol.freeze(),
                })));
            } else if frame == b"na\n"[..] {
                return Ok(Async::Ready(Some(ListenerToDialerMessage::NotAvailable)));
            } else {
                // A varint number of protocols
                let (num_protocols, mut remaining) = decode::usize(&frame)?;
                if num_protocols > 1000 { // TODO: configurable limit
                    return Err(MultistreamSelectError::VarintParseError("too many protocols".into()))
                }
                remaining = &remaining[1 ..]; // skip newline after num_protocols
                let mut out = Vec::with_capacity(num_protocols);
                for _ in 0 .. num_protocols {
                    let (len, rem) = decode::usize(remaining)?;
                    if len == 0 || len > rem.len() || rem[len - 1] != b'\n' {
                        return Err(MultistreamSelectError::UnknownMessage)
                    }
                    out.push(Bytes::from(&rem[.. len - 1]));
                    remaining = &rem[len ..]
                }
                return Ok(Async::Ready(Some(
                    ListenerToDialerMessage::ProtocolsListResponse { list: out },
                )));
            }
        }
    }
}

/// Future, returned by `Dialer::new`, which send the handshake and returns the actual `Dialer`.
pub struct DialerFuture<T: AsyncWrite, N> {
    inner: sink::Send<Framed<T, UviBytes<RawSlice>>>,
    _mark: std::marker::PhantomData<N>
}

impl<T: AsyncWrite, N> Future for DialerFuture<T, N> {
    type Item = Dialer<T, N>;
    type Error = MultistreamSelectError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let inner = try_ready!(self.inner.poll());
        Ok(Async::Ready(Dialer {
            inner,
            handshake_finished: false,
            _mark: std::marker::PhantomData
        }))
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::{Dialer, DialerToListenerMessage, MultistreamSelectError};
    use tokio::runtime::current_thread::Runtime;
    use tokio_tcp::{TcpListener, TcpStream};
    use futures::Future;
    use futures::{Sink, Stream};

    #[test]
    fn wrong_proto_name() {
        let listener = TcpListener::bind(&"127.0.0.1:0".parse().unwrap()).unwrap();
        let listener_addr = listener.local_addr().unwrap();

        let server = listener
            .incoming()
            .into_future()
            .map(|_| ())
            .map_err(|(e, _)| e.into());

        let client = TcpStream::connect(&listener_addr)
            .from_err()
            .and_then(move |stream| Dialer::new(stream))
            .and_then(move |dialer| {
                let p = b"invalid_name";
                dialer.send(DialerToListenerMessage::ProtocolRequest { name: p })
            });

        let mut rt = Runtime::new().unwrap();
        match rt.block_on(server.join(client)) {
            Err(MultistreamSelectError::WrongProtocolName) => (),
            _ => panic!(),
        }
    }
}
