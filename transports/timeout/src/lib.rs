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

//! Wraps around a `Transport` and adds a timeout to all the incoming and outgoing connections.
//!
//! The timeout includes the upgrading process.
// TODO: add example

#[macro_use]
extern crate futures;
extern crate libp2p_core;
#[macro_use]
extern crate log;
extern crate tokio_timer;

use futures::{Async, Future, Poll, Stream};
use libp2p_core::{Multiaddr, Transport};
use std::{fmt, io, time::Duration};
use tokio_timer::Timeout;

/// Wraps around a `Transport` and adds a timeout to all the incoming and outgoing connections.
///
/// The timeout includes the upgrade. There is no timeout on the listener or on stream of incoming
/// substreams.
#[derive(Debug, Copy, Clone)]
pub struct TransportTimeout<InnerTrans> {
    inner: InnerTrans,
    outgoing_timeout: Duration,
    incoming_timeout: Duration,
}

impl<InnerTrans> TransportTimeout<InnerTrans> {
    /// Wraps around a `Transport` to add timeouts to all the sockets created by it.
    #[inline]
    pub fn new(trans: InnerTrans, timeout: Duration) -> Self {
        TransportTimeout {
            inner: trans,
            outgoing_timeout: timeout,
            incoming_timeout: timeout,
        }
    }

    /// Wraps around a `Transport` to add timeouts to the outgoing connections.
    #[inline]
    pub fn with_outgoing_timeout(trans: InnerTrans, timeout: Duration) -> Self {
        TransportTimeout {
            inner: trans,
            outgoing_timeout: timeout,
            incoming_timeout: Duration::from_secs(100 * 365 * 24 * 3600), // 100 years
        }
    }

    /// Wraps around a `Transport` to add timeouts to the ingoing connections.
    #[inline]
    pub fn with_ingoing_timeout(trans: InnerTrans, timeout: Duration) -> Self {
        TransportTimeout {
            inner: trans,
            outgoing_timeout: Duration::from_secs(100 * 365 * 24 * 3600), // 100 years
            incoming_timeout: timeout,
        }
    }
}

impl<InnerTrans> Transport for TransportTimeout<InnerTrans>
where
    InnerTrans: Transport,
{
    type Output = InnerTrans::Output;
    type Error = Error<InnerTrans::Error>;
    type Listener = TimeoutListener<InnerTrans::Listener>;
    type ListenerUpgrade = TokioTimerMapErr<InnerTrans::ListenerUpgrade>;
    type Dial = TokioTimerMapErr<InnerTrans::Dial>;

    fn listen_on(self, addr: Multiaddr) -> Result<(Self::Listener, Multiaddr), (Self, Multiaddr)> {
        match self.inner.listen_on(addr) {
            Ok((listener, addr)) => {
                let listener = TimeoutListener {
                    inner: listener,
                    timeout: self.incoming_timeout,
                };

                Ok((listener, addr))
            }
            Err((inner, addr)) => {
                let transport = TransportTimeout {
                    inner,
                    outgoing_timeout: self.outgoing_timeout,
                    incoming_timeout: self.incoming_timeout,
                };

                Err((transport, addr))
            }
        }
    }

    fn dial(self, addr: Multiaddr) -> Result<Self::Dial, (Self, Multiaddr)> {
        match self.inner.dial(addr) {
            Ok(dial) => Ok(TokioTimerMapErr {
                inner: Timeout::new(dial, self.outgoing_timeout),
            }),
            Err((inner, addr)) => {
                let transport = TransportTimeout {
                    inner,
                    outgoing_timeout: self.outgoing_timeout,
                    incoming_timeout: self.incoming_timeout,
                };

                Err((transport, addr))
            }
        }
    }

    #[inline]
    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        self.inner.nat_traversal(server, observed)
    }
}

// TODO: can be removed and replaced with an `impl Stream` once impl Trait is fully stable
//       in Rust (https://github.com/rust-lang/rust/issues/34511)
pub struct TimeoutListener<InnerStream> {
    inner: InnerStream,
    timeout: Duration,
}

impl<InnerStream, O> Stream for TimeoutListener<InnerStream>
where
    InnerStream: Stream<Item = (O, Multiaddr), Error = io::Error>,
{
    type Item = (TokioTimerMapErr<O>, Multiaddr);
    type Error = InnerStream::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        let poll_out = try_ready!(self.inner.poll());
        if let Some((inner_fut, addr)) = poll_out {
            let fut = TokioTimerMapErr {
                inner: Timeout::new(inner_fut, self.timeout),
            };
            Ok(Async::Ready(Some((fut, addr))))
        } else {
            Ok(Async::Ready(None))
        }
    }
}

/// Wraps around a `Future`. Turns the error type from `TimeoutError<io::Error>` to `Error`.
// TODO: can be replaced with `impl Future` once `impl Trait` are fully stable in Rust
//       (https://github.com/rust-lang/rust/issues/34511)
#[must_use = "futures do nothing unless polled"]
pub struct TokioTimerMapErr<InnerFut> {
    inner: Timeout<InnerFut>
}

impl<InnerFut> Future for TokioTimerMapErr<InnerFut>
where
    InnerFut: Future
{
    type Item = InnerFut::Item;
    type Error = Error<InnerFut::Error>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.inner.poll().map_err(|err| {
            if err.is_inner() {
                Error::Transport(err.into_inner().expect("ensured by is_inner()"))
            } else if err.is_elapsed() {
                debug!("timeout elapsed for connection");
                Error::Timeout
            } else {
                assert!(err.is_timer());
                debug!("tokio timer error in timeout wrapper");
                Error::Timer
            }
        })
    }
}

#[derive(Debug)]
pub enum Error<E> {
    Transport(E),
    Timeout,
    Timer
}

impl<E> fmt::Display for Error<E>
where
    E: fmt::Display
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Transport(e) => write!(f, "transport error: {}", e),
            Error::Timeout => f.write_str("timeout"),
            Error::Timer => f.write_str("timer error")
        }
    }
}

impl<E> std::error::Error for Error<E>
where
    E: std::error::Error
{
    fn cause(&self) -> Option<&dyn std::error::Error> {
        match self {
            Error::Transport(e) => Some(e),
            Error::Timeout | Error::Timer => None
        }
    }
}

