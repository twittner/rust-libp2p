// Copyright 2019 Parity Technologies (UK) Ltd.
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

use bytes::BytesMut;
use futures::{prelude::*, try_ready};
use log::trace;

#[derive(Debug, Clone)]
pub struct Buffered<S> {
    sink: S,
    buf: BytesMut,
    max: usize
}

impl<S: Sink<SinkItem=BytesMut>> Buffered<S> {
    pub fn new(sink: S, size: usize) -> Self {
        trace!("buffer = {}", size);
        Buffered {
            sink,
            buf: BytesMut::new(),
            max: size
        }
    }

    fn flush_buffer(&mut self) -> Poll<(), S::SinkError> {
        trace!("flush_buffer: {}", self.buf.len());
        let buf = std::mem::replace(&mut self.buf, BytesMut::new());
        if let AsyncSink::NotReady(buf) = self.sink.start_send(buf)? {
            std::mem::replace(&mut self.buf, buf);
            Ok(Async::NotReady)
        } else {
            Ok(Async::Ready(()))
        }
    }
}

impl<S: Sink<SinkItem=BytesMut>> Sink for Buffered<S> {
    type SinkItem = S::SinkItem;
    type SinkError = S::SinkError;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        trace!("start_send: {}", item.len());
        loop {
            if self.buf.is_empty() && item.len() > self.max {
                return self.sink.start_send(item)
            }
            if self.buf.len() + item.len() <= self.max {
                self.buf.extend_from_slice(&item[..]);
                return Ok(AsyncSink::Ready)
            }
            if self.flush_buffer()?.is_not_ready() {
                return Ok(AsyncSink::NotReady(item))
            }
        }
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        try_ready!(self.flush_buffer());
        self.sink.poll_complete()
    }
}

impl<S: Sink + Stream> Stream for Buffered<S> {
    type Item = S::Item;
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Option<S::Item>, S::Error> {
        self.sink.poll()
    }
}

