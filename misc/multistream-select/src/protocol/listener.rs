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

//! Contains the `Listener` wrapper, which allows raw communications with a dialer.

use bytes::Bytes;
use futures::prelude::*;
use crate::protocol::DialerToListenerMessage;
use crate::protocol::ListenerToDialerMessage;
use crate::protocol::MultistreamSelectError;
use crate::protocol::{MULTISTREAM_PROTOCOL, MULTISTREAM_PROTOCOL_WITH_LF};
use crate::protocol::Aio;
use log::{debug, trace};
use tokio_codec::FramedRead;
use tokio_io::{AsyncRead, AsyncWrite, io::{ReadHalf, WriteHalf}};
use unsigned_varint::{encode, codec::UviBytes, io::UviWriter};

/// Wraps around a `AsyncRead+AsyncWrite`.
/// Assumes that we're on the listener's side. Produces and accepts messages.
pub struct Listener<R, N> {
    stream: FramedRead<ReadHalf<R>, UviBytes>,
    sink: UviWriter<WriteHalf<R>>,
    state: State,
    _mark: std::marker::PhantomData<N>
}

impl<R, N> Listener<R, N>
where
    R: AsyncRead + AsyncWrite,
{
    /// Takes ownership of a socket and starts the handshake. If the handshake succeeds, the
    /// future returns a `Listener`.
    pub fn new(inner: R) -> ListenerFuture<R, N> {
        let (reader, writer) = inner.split();
        let mut codec = UviBytes::default();
        codec.set_max_len(std::u16::MAX as usize);
        let mut writer = UviWriter::new(writer);
        writer.set_suffix(b"\n");
        let listener = Listener {
            stream: FramedRead::new(reader, codec),
            sink: writer,
            state: State::Start,
            _mark: std::marker::PhantomData
        };
        ListenerFuture {
            listener: Some(listener),
            state: ListenerFutureState::Await
        }
    }

    /// Grants back the socket. Typically used after a `ProtocolRequest` has been received and a
    /// `ProtocolAck` has been sent back.
    #[inline]
    pub fn into_inner(self) -> Aio<R> {
        Aio(self.stream.into_inner(), self.sink.into_inner())
    }
}

#[derive(Debug)]
enum State {
    Start,
    Message(usize),
    List(Vec<u8>, usize)
}

impl<R, N> Sink for Listener<R, N>
where
    R: AsyncRead + AsyncWrite,
    N: AsRef<[u8]>
{
    type SinkItem = ListenerToDialerMessage<N>;
    type SinkError = MultistreamSelectError;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        use crate::protocol::ListenerToDialerMessage::*;
        loop {
            match (&item, &mut self.state) {
                (ProtocolsListResponse { list }, State::Start) => {
                    let mut buf = encode::usize_buffer();
                    let mut out_msg = Vec::from(encode::usize(list.len(), &mut buf));
                    out_msg.push(b'\n');
                    for i in 0 .. list.len() {
                        out_msg.extend(encode::usize(list[i].as_ref().len() + 1, &mut buf)); // +1 for '\n'
                        out_msg.extend_from_slice(list[i].as_ref());
                        if i < list.len() - 1 {
                            out_msg.push(b'\n')
                        }
                    }
                    self.state = State::List(out_msg, 0)
                }
                (_, State::Start) => {
                    self.state = State::Message(0)
                }
                (ProtocolAck { name }, State::Message(offset)) => {
                    if !name.as_ref().starts_with(b"/") {
                        debug!("invalid protocol name {:?}", name.as_ref());
                        return Err(MultistreamSelectError::WrongProtocolName);
                    }
                    match self.sink.poll_write(&name.as_ref()[*offset ..])? {
                        Async::Ready(n) => {
                            *offset += n;
                            if *offset >= name.as_ref().len() {
                                self.state = State::Start;
                                return Ok(AsyncSink::Ready)
                            }
                        }
                        Async::NotReady => return Ok(AsyncSink::NotReady(item))
                    }
                }
                (NotAvailable, State::Message(offset)) => {
                    match self.sink.poll_write(&b"na"[*offset ..])? {
                        Async::Ready(n) => {
                            *offset += n;
                            if *offset >= 2 {
                                self.state = State::Start;
                                return Ok(AsyncSink::Ready)
                            }
                        }
                        Async::NotReady => return Ok(AsyncSink::NotReady(item))
                    }
                }
                (ProtocolsListResponse {..}, State::List(list, offset)) => {
                    match self.sink.poll_write(&list[*offset ..])? {
                        Async::Ready(n) => {
                            *offset += n;
                            if *offset >= list.len() {
                                self.state = State::Start;
                                return Ok(AsyncSink::Ready)
                            }
                        }
                        Async::NotReady => return Ok(AsyncSink::NotReady(item))
                    }
                }
                | (ProtocolAck {..}, State::List(..))
                | (NotAvailable, State::List(..))
                | (ProtocolsListResponse {..}, State::Message(..)) => {
                    unreachable!("invalid state combination")
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

impl<R, N> Stream for Listener<R, N>
where
    R: AsyncRead + AsyncWrite
{
    type Item = DialerToListenerMessage<Bytes>;
    type Error = MultistreamSelectError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        let mut frame = match self.stream.poll() {
            Ok(Async::Ready(Some(frame))) => frame,
            Ok(Async::Ready(None)) => return Ok(Async::Ready(None)),
            Ok(Async::NotReady) => return Ok(Async::NotReady),
            Err(err) => return Err(err.into()),
        };

        if frame.get(0) == Some(&b'/') && frame.last() == Some(&b'\n') {
            let frame_len = frame.len();
            let protocol = frame.split_to(frame_len - 1);
            Ok(Async::Ready(Some(
                DialerToListenerMessage::ProtocolRequest { name: protocol.freeze() },
            )))
        } else if frame == b"ls\n"[..] {
            Ok(Async::Ready(Some(
                DialerToListenerMessage::ProtocolsListRequest,
            )))
        } else {
            Err(MultistreamSelectError::UnknownMessage)
        }
    }
}


/// Future, returned by `Listener::new` which performs the handshake and returns
/// the `Listener` if successful.
pub struct ListenerFuture<R, N> {
    listener: Option<Listener<R, N>>,
    state: ListenerFutureState
}

enum ListenerFutureState {
    Await,
    Reply(usize)
}

impl<R: AsyncRead + AsyncWrite, N> Future for ListenerFuture<R, N> {
    type Item = Listener<R, N>;
    type Error = MultistreamSelectError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let mut listener = self.listener.take().unwrap();
        loop {
            match self.state {
                ListenerFutureState::Await => {
                    let msg = match listener.stream.poll()? {
                        Async::Ready(msg) => msg,
                        Async::NotReady => {
                            self.listener = Some(listener);
                            return Ok(Async::NotReady)
                        }
                    };
                    if msg.as_ref().map(|m| m.as_ref()) != Some(MULTISTREAM_PROTOCOL_WITH_LF) {
                        debug!("failed handshake; received: {:?}", msg);
                        return Err(MultistreamSelectError::FailedHandshake)
                    }
                    trace!("sending back /multistream/<version> to finish the handshake");
                    self.state = ListenerFutureState::Reply(0)
                }
                ListenerFutureState::Reply(ref mut offset) => {
                    if *offset >= MULTISTREAM_PROTOCOL.len() {
                        return Ok(Async::Ready(listener))
                    }
                    match listener.sink.poll_write(&MULTISTREAM_PROTOCOL[*offset ..])? {
                        Async::Ready(n) => { *offset += n }
                        Async::NotReady => {
                            self.listener = Some(listener);
                            return Ok(Async::NotReady)
                        }
                    }
                }
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use tokio::runtime::current_thread::Runtime;
    use tokio_tcp::{TcpListener, TcpStream};
    use bytes::Bytes;
    use futures::Future;
    use futures::{Sink, Stream};
    use crate::protocol::{Dialer, Listener, ListenerToDialerMessage, MultistreamSelectError};

    #[test]
    fn wrong_proto_name() {
        let listener = TcpListener::bind(&"127.0.0.1:0".parse().unwrap()).unwrap();
        let listener_addr = listener.local_addr().unwrap();

        let server = listener
            .incoming()
            .into_future()
            .map_err(|(e, _)| e.into())
            .and_then(move |(connec, _)| Listener::<_, Bytes>::new(connec.unwrap()))
            .and_then(|listener| {
                let proto_name = Bytes::from("invalid-proto");
                listener.send(ListenerToDialerMessage::ProtocolAck { name: proto_name })
            });

        let client = TcpStream::connect(&listener_addr)
            .from_err()
            .and_then(move |stream| Dialer::<_, Bytes>::new(stream));

        let mut rt = Runtime::new().unwrap();
        match rt.block_on(server.join(client)) {
            Err(MultistreamSelectError::WrongProtocolName) => (),
            _ => panic!(),
        }
    }
}
