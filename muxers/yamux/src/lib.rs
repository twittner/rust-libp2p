// Copyright 2018 Parity Technologies (UK) Ltd.
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

//! Implements the Yamux multiplexing protocol for libp2p, see also the
//! [specification](https://github.com/hashicorp/yamux/blob/master/spec.md).

use futures::{future::{self, FutureResult}, prelude::*};
use libp2p_core::upgrade::{InboundUpgrade, OutboundUpgrade, UpgradeInfo, Negotiated};
use parking_lot::Mutex;
use std::{io, iter, sync::{Arc, atomic}};
use tokio_executor::DefaultExecutor;
use tokio_io::{AsyncRead, AsyncWrite};

/// Yamux connection.
pub struct Yamux<C> {
    inner: Arc<Mutex<Inner<C>>>,
    init: atomic::AtomicBool,
}

struct Inner<C> {
    handle: yamux::Handle,
    close: Closing,
    _mark: std::marker::PhantomData<C>
}

/// States during connection closing.
enum Closing {
    /// Connection is open.
    NotStarted,
    /// Connection close is in progress.
    InProgress(Box<dyn Future<Item = (), Error = io::Error> + Send>),
    /// All streams have been marked as closed.
    Done
}

impl<C> Yamux<C>
where
    C: AsyncRead + AsyncWrite + Send + 'static
{
    pub fn new(c: C, mut cfg: yamux::Config, mode: yamux::Mode) -> Result<Self, io::Error> {
        let e = DefaultExecutor::current();

        cfg.set_read_after_close(false);
        let h = yamux::Connection::new(e, c, cfg, mode).map_err(|e| {
            io::Error::new(io::ErrorKind::Other, e)
        })?;

        Ok(Yamux {
            inner: Arc::new(Mutex::new(Inner {
                handle: h,
                close: Closing::NotStarted,
                _mark: std::marker::PhantomData
            })),
            init: atomic::AtomicBool::new(false)
        })
     }
 }

impl<C> libp2p_core::StreamMuxer for Yamux<C>
where
    C: AsyncRead + AsyncWrite + Send + 'static
{
    type Substream = yamux::Stream;
    type OutboundSubstream = Box<dyn Future<Item = Option<Self::Substream>, Error = io::Error> + Send>;
    type Error = io::Error;

    fn poll_inbound(&self) -> Poll<Self::Substream, io::Error> {
        match self.inner.lock().handle.poll() {
            Err(()) => Err(io::Error::new(io::ErrorKind::Other, "connection gone")),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Ok(Async::Ready(None)) => Err(io::ErrorKind::BrokenPipe.into()),
            Ok(Async::Ready(Some(stream))) => {
                self.init.store(true, atomic::Ordering::Release);
                Ok(Async::Ready(stream))
            }
        }
    }

    fn open_outbound(&self) -> Self::OutboundSubstream {
        Box::new(self.inner.lock().handle.open_stream()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e)))
    }

    fn poll_outbound(&self, substream: &mut Self::OutboundSubstream) -> Poll<Self::Substream, io::Error> {
        match substream.poll()? {
            Async::Ready(Some(s)) => Ok(Async::Ready(s)),
            Async::Ready(None) => Err(io::ErrorKind::BrokenPipe.into()),
            Async::NotReady => Ok(Async::NotReady),
        }
    }

    fn destroy_outbound(&self, _: Self::OutboundSubstream) { }

    fn read_substream(&self, sub: &mut Self::Substream, buf: &mut [u8]) -> Poll<usize, io::Error> {
        let result = sub.poll_read(buf);
        if let Ok(Async::Ready(_)) = result {
            self.init.store(true, atomic::Ordering::Release);
        }
        result
    }

    fn write_substream(&self, sub: &mut Self::Substream, buf: &[u8]) -> Poll<usize, io::Error> {
        sub.poll_write(buf)
    }

    fn flush_substream(&self, sub: &mut Self::Substream) -> Poll<(), io::Error> {
        sub.poll_flush()
    }

    fn shutdown_substream(&self, sub: &mut Self::Substream) -> Poll<(), io::Error> {
        sub.shutdown()
    }

    fn destroy_substream(&self, _: Self::Substream) { }

    fn is_remote_acknowledged(&self) -> bool {
        self.init.load(atomic::Ordering::Acquire)
    }

    fn close(&self) -> Poll<(), io::Error> {
        let mut inner = self.inner.lock();
        match inner.close {
            Closing::NotStarted => {
                let mut f = inner.handle.close().map_err(|e| io::Error::new(io::ErrorKind::Other, e));
                if f.poll()?.is_ready() {
                    inner.close = Closing::Done;
                    Ok(Async::Ready(()))
                } else {
                    inner.close = Closing::InProgress(Box::new(f));
                    Ok(Async::NotReady)
                }
            }
            Closing::InProgress(ref mut f) => {
                if f.poll()?.is_ready() {
                    inner.close = Closing::Done;
                    Ok(Async::Ready(()))
                } else {
                    Ok(Async::NotReady)
                }
            }
            Closing::Done => Ok(Async::Ready(()))
        }
    }

    fn flush_all(&self) -> Poll<(), io::Error> {
        Ok(Async::Ready(()))
    }
}

#[derive(Clone)]
pub struct Config(yamux::Config);

impl Config {
    pub fn new(cfg: yamux::Config) -> Self {
        Config(cfg)
    }
}

impl Default for Config {
    fn default() -> Self {
        Config(yamux::Config::default())
    }
}

impl UpgradeInfo for Config {
    type Info = &'static [u8];
    type InfoIter = iter::Once<Self::Info>;

    fn protocol_info(&self) -> Self::InfoIter {
        iter::once(b"/yamux/1.0.0")
    }
}

impl<C> InboundUpgrade<C> for Config
where
    C: AsyncRead + AsyncWrite + Send + 'static
{
    type Output = Yamux<Negotiated<C>>;
    type Error = io::Error;
    type Future = FutureResult<Yamux<Negotiated<C>>, io::Error>;

    fn upgrade_inbound(self, i: Negotiated<C>, _: Self::Info) -> Self::Future {
        future::result(Yamux::new(i, self.0, yamux::Mode::Server))
    }
}

impl<C> OutboundUpgrade<C> for Config
where
    C: AsyncRead + AsyncWrite + Send + 'static
{
    type Output = Yamux<Negotiated<C>>;
    type Error = io::Error;
    type Future = FutureResult<Yamux<Negotiated<C>>, io::Error>;

    fn upgrade_outbound(self, i: Negotiated<C>, _: Self::Info) -> Self::Future {
        future::result(Yamux::new(i, self.0, yamux::Mode::Client))
    }
}

