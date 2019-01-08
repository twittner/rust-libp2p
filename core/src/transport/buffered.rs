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

use crate::transport::Transport;
use futures::prelude::*;
use multiaddr::Multiaddr;
use std::io;
use tokio_io::{AsyncRead, AsyncWrite, io::{ReadHalf, WriteHalf}};

#[derive(Debug, Clone)]
pub struct BufferedTransport<T> {
    transport: T,
    bufsize: usize
}

impl<T: Transport> BufferedTransport<T> {
    pub fn new(transport: T, bufsize: usize) -> Self {
        BufferedTransport { transport, bufsize }
    }
}

impl<T: Transport> Transport for BufferedTransport<T>
where
    T::Output: AsyncRead + AsyncWrite
{
    type Output = Buffered<T::Output>;
    type Listener = BufferedListener<T::Listener>;
    type ListenerUpgrade = ApplyBuffer<T::ListenerUpgrade>;
    type Dial = ApplyBuffer<T::Dial>;

    fn listen_on(self, addr: Multiaddr) -> Result<(Self::Listener, Multiaddr), (Self, Multiaddr)> {
        match self.transport.listen_on(addr) {
            Ok((listener, addr)) => {
                let listener = BufferedListener { listener, bufsize: self.bufsize };
                Ok((listener, addr))
            }
            Err((transport, addr)) => {
                let transport = BufferedTransport { transport, bufsize: self.bufsize };
                Err((transport, addr))
            }
        }
    }

    fn dial(self, addr: Multiaddr) -> Result<Self::Dial, (Self, Multiaddr)> {
        match self.transport.dial(addr) {
            Ok(dial) => Ok(ApplyBuffer { future: dial, bufsize: self.bufsize }),
            Err((transport, addr)) => {
                let transport = BufferedTransport { transport, bufsize: self.bufsize };
                Err((transport, addr))
            }
        }
    }

    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        self.transport.nat_traversal(server, observed)
    }
}

#[derive(Debug, Clone)]
pub struct BufferedListener<T> {
    listener: T,
    bufsize: usize
}

impl<S, T> Stream for BufferedListener<S>
where
    S: Stream<Item = (T, Multiaddr)>
{
    type Item = (ApplyBuffer<T>, Multiaddr);
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if let Some((future, addr)) = try_ready!(self.listener.poll()) {
            let future = ApplyBuffer { future, bufsize: self.bufsize };
            Ok(Async::Ready(Some((future, addr))))
        } else {
            Ok(Async::Ready(None))
        }
    }
}

#[derive(Debug, Clone)]
pub struct ApplyBuffer<T> {
    future: T,
    bufsize: usize
}

impl<T: Future> Future for ApplyBuffer<T>
where
    T::Item: AsyncRead + AsyncWrite
{
    type Item = Buffered<T::Item>;
    type Error = T::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let output = try_ready!(self.future.poll());
        let (r, w) = output.split();
        let br = io::BufReader::with_capacity(self.bufsize, r);
        let bw = io::BufWriter::with_capacity(self.bufsize, w);
        Ok(Async::Ready(Buffered { reader: br, writer: bw }))
    }
}

#[derive(Debug)]
pub struct Buffered<T: AsyncRead + AsyncWrite> {
    reader: io::BufReader<ReadHalf<T>>,
    writer: io::BufWriter<WriteHalf<T>>
}

impl<T: AsyncRead + AsyncWrite> io::Read for Buffered<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reader.read(buf)
    }
}

impl<T: AsyncRead + AsyncWrite> AsyncRead for Buffered<T> { }

impl<T: AsyncRead + AsyncWrite> io::Write for Buffered<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writer.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

impl<T: AsyncRead + AsyncWrite> AsyncWrite for Buffered<T> {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.writer.shutdown()
    }
}
