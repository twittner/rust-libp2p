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

use futures::prelude::*;
use multiaddr::Multiaddr;
use transport::Transport;

/// See `Transport::map_err`.
#[derive(Debug, Copy, Clone)]
pub struct MapErr<T, F> {
    transport: T,
    fun: F,
}

impl<T, F> MapErr<T, F> {
    /// Internal function that builds a `MapErr`.
    #[inline]
    pub(crate) fn new(transport: T, fun: F) -> MapErr<T, F> {
        MapErr { transport, fun }
    }
}

impl<T, F, E> Transport for MapErr<T, F>
where
    T: Transport,
    F: FnOnce(T::Error) -> E + Clone,
{
    type Output = T::Output;
    type Error = E;
    type Listener = MapErrListener<T, F>;
    type ListenerUpgrade = MapErrListenerUpgrade<T, F>;
    type Dial = MapErrDial<T, F, E>;

    fn listen_on(self, addr: Multiaddr) -> Result<(Self::Listener, Multiaddr), (Self, Multiaddr)> {
        match self.transport.listen_on(addr) {
            Ok((stream, listen_addr)) => {
                let stream = MapErrListener { inner: stream, fun: self.fun };
                Ok((stream, listen_addr))
            }
            Err((transport, addr)) => Err((MapErr { transport, fun: self.fun }, addr)),
        }
    }

    fn dial(self, addr: Multiaddr) -> Result<Self::Dial, (Self, Multiaddr)> {
        match self.transport.dial(addr) {
            Ok(future) => Ok(MapErrDial {
                inner: future,
                fun: Some(self.fun)
            }),
            Err((transport, addr)) => Err((MapErr { transport, fun: self.fun }, addr)),
        }
    }

    #[inline]
    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        self.transport.nat_traversal(server, observed)
    }
}

/// Listening stream for `MapErr`.
pub struct MapErrListener<T, F>
where T: Transport {
    inner: T::Listener,
    fun: F,
}

impl<T, F, E> Stream for MapErrListener<T, F>
where
    T: Transport,
    F: FnOnce(T::Error) -> E + Clone
{
    type Item = (MapErrListenerUpgrade<T, F>, Multiaddr);
    type Error = std::io::Error;

    #[inline]
    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match try_ready!(self.inner.poll()) {
            Some((value, addr)) => {
                let up = MapErrListenerUpgrade {
                    inner: value,
                    fun: Some(self.fun.clone())
                };
                Ok(Async::Ready(Some((up, addr))))
            }
            None => Ok(Async::Ready(None))
        }
    }
}

/// Listening upgrade future for `MapErr`.
pub struct MapErrListenerUpgrade<T, F>
where T: Transport {
    inner: T::ListenerUpgrade,
    fun: Option<F>,
}

impl<T, F, E> Future for MapErrListenerUpgrade<T, F>
where
    T: Transport,
    F: FnOnce(T::Error) -> E
{
    type Item = T::Output;
    type Error = E;

    #[inline]
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.inner.poll() {
            Ok(Async::Ready(value)) => {
                Ok(Async::Ready(value))
            },
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(err) => {
                let fun = self.fun.take().expect("poll() called again after error");
                Err(fun(err))
            }
        }
    }
}

/// Dialing future for `MapErr`.
pub struct MapErrDial<T, F, E>
where
    T: Transport,
    F: FnOnce(T::Error) -> E
{
    inner: T::Dial,
    fun: Option<F>,
}

impl<T, F, E> Future for MapErrDial<T, F, E>
where
    T: Transport,
    F: FnOnce(T::Error) -> E
{
    type Item = T::Output;
    type Error = E;

    #[inline]
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.inner.poll() {
            Ok(Async::Ready(value)) => Ok(Async::Ready(value)),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(err) => {
                let fun = self.fun.take().expect("poll() called again after error");
                Err(fun(err))
            }
        }
    }
}
