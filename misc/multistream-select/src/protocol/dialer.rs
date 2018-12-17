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
use crate::protocol::MULTISTREAM_PROTOCOL_WITH_LF;
use crate::protocol::Aio;
use futures::{prelude::*, try_ready};
use tokio_codec::FramedRead;
use tokio_io::{AsyncRead, AsyncWrite, io::{ReadHalf, WriteHalf}};
use unsigned_varint::{decode, codec::{UviBytes, UviWriter}};

/// Wraps around a `AsyncRead + AsyncWrite`.
/// Assumes that we're on the dialer's side. Produces and accepts messages.
pub struct Dialer<R, N> {
    stream: FramedRead<ReadHalf<R>, UviBytes>,
    sink: UviWriter<WriteHalf<R>>,
    handshake_finished: bool,
    state: State<N>
}

enum State<N> {
    Start,
    Message {
        item: DialerToListenerMessage<N>,
        offset: usize
    },
    Newline
}

impl<R, N> Dialer<R, N>
where
    R: AsyncRead + AsyncWrite
{
    pub fn new(inner: R) -> DialerFuture<R, N> {
        let (reader, writer) = inner.split();
        let mut codec = UviBytes::default();
        codec.set_max_len(std::u16::MAX as usize);
        let dialer = Dialer {
            stream: FramedRead::new(reader, codec),
            sink: UviWriter::new(writer),
            handshake_finished: false,
            state: State::Start
        };
        DialerFuture { dialer, offset: 0 }

    }

    /// Grants back the socket. Typically used after a `ProtocolAck` has been received.
    #[inline]
    pub fn into_inner(self) -> Aio<R> {
        Aio(self.stream.into_inner(), self.sink.into_inner())
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
        use crate::protocol::DialerToListenerMessage::*;
        loop {
            match self.state {
                State::Start => {
                    self.state = State::Message { item, offset: 0 }
                }
                State::Message { item: ProtocolRequest { name }, ref mut offset } => {
                    if !name.as_ref().starts_with(b"/") {
                        return Err(MultistreamSelectError::WrongProtocolName);
                    }
                    match self.sink.poll_write(&name.as_ref()[*offset ..])? {
                        Async::Ready(n) => {
                            *offset += n;
                            if *offset >= name.as_ref().len() {
                                self.state = State::Newline
                            }
                        }
                        Async::NotReady => return Ok(AsyncSink::NotReady(item))
                    }
                }
                State::Message { item: ProtocolsListRequest, ref mut offset } => {
                    match self.sink.poll_write(&b"ls\n"[*offset ..])? {
                        Async::Ready(n) => {
                            *offset += n;
                            if *offset >= 3 {
                                self.state = State::Start
                            }
                        }
                        Async::NotReady => return Ok(AsyncSink::NotReady(item))
                    }
                }
                State::Newline => {
                    match self.sink.poll_write(b"\n")? {
                        Async::Ready(0) => return Ok(AsyncSink::NotReady(item)),
                        Async::Ready(_) => {
                            self.state = State::Start
                        }
                        Async::NotReady => return Ok(AsyncSink::NotReady(item))
                    }
                }
            }
        }
    }

    #[inline]
    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        Ok(self.sink.poll_flush()?)
    }

    #[inline]
    fn close(&mut self) -> Poll<(), Self::SinkError> {
        Ok(self.sink.shutdown()?)
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
            let mut frame = match self.stream.poll() {
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
pub struct DialerFuture<R, N> {
    dialer: Dialer<R, N>,
    offset: usize
}

impl<R: AsyncWrite, N> Future for DialerFuture<R, N> {
    type Item = Dialer<R, N>;
    type Error = MultistreamSelectError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            if self.offset >= MULTISTREAM_PROTOCOL_WITH_LF.len() {
                try_ready!(self.dialer.sink.poll_flush());
                return Ok(Async::Ready(self.dialer))
            } else {
                match self.dialer.sink.poll_write(&MULTISTREAM_PROTOCOL_WITH_LF[self.offset ..])? {
                    Async::Ready(n) => { self.offset += n }
                    Async::NotReady => return Ok(Async::NotReady)
                }
            }
        }
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
