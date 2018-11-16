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
use crate::transport::{Dialer, Listener};

#[derive(Debug, Copy, Clone)]
pub struct MapErrDialer<T, F> { inner: T, fun: F }

impl<T, F> MapErrDialer<T, F> {
    pub fn new(inner: T, fun: F) -> Self {
        MapErrDialer { inner, fun }
    }
}

impl<L, F> Listener for MapErrDialer<L, F>
where
    L: Listener
{
    type Output = L::Output;
    type Error = L::Error;
    type Inbound = L::Inbound;
    type Upgrade = L::Upgrade;

    fn listen_on(self, addr: Multiaddr) -> Result<(Self::Inbound, Multiaddr), (Self, Multiaddr)> {
        let fun = self.fun;
        self.inner.listen_on(addr)
            .map_err(move |(listener, addr)| (MapErrDialer::new(listener, fun), addr))
    }

    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        self.inner.nat_traversal(server, observed)
    }
}

impl<D, F, E> Dialer for MapErrDialer<D, F>
where
    D: Dialer,
    F: FnOnce(D::Error, Multiaddr) -> E
{
    type Output = D::Output;
    type Error = E;
    type Outbound = MapErrFuture<D::Outbound, F>;

    fn dial(self, addr: Multiaddr) -> Result<Self::Outbound, (Self, Multiaddr)> {
        match self.inner.dial(addr.clone()) {
            Ok(future) => Ok(MapErrFuture {
                fut: future,
                args: Some((self.fun, addr))
            }),
            Err((dialer, addr)) => Err((MapErrDialer::new(dialer, self.fun), addr))
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct MapErrListener<T, F> { inner: T, fun: F }

impl<T, F> MapErrListener<T, F> {
    pub fn new(inner: T, fun: F) -> Self {
        MapErrListener { inner, fun }
    }
}

impl<L, F, E> Listener for MapErrListener<L, F>
where
    L: Listener,
    F: FnOnce(L::Error, Multiaddr) -> E + Clone
{
    type Output = L::Output;
    type Error = E;
    type Inbound = MapErrStream<L::Inbound, F>;
    type Upgrade = MapErrFuture<L::Upgrade, F>;

    fn listen_on(self, addr: Multiaddr) -> Result<(Self::Inbound, Multiaddr), (Self, Multiaddr)> {
        match self.inner.listen_on(addr) {
            Ok((stream, addr)) => {
                let stream = MapErrStream { stream, fun: self.fun };
                Ok((stream, addr))
            }
            Err((listener, addr)) => Err((MapErrListener::new(listener, self.fun), addr)),
        }
    }

    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        self.inner.nat_traversal(server, observed)
    }
}

impl<D, F> Dialer for MapErrListener<D, F>
where
    D: Dialer
{
    type Output = D::Output;
    type Error = D::Error;
    type Outbound = D::Outbound;

    fn dial(self, addr: Multiaddr) -> Result<Self::Outbound, (Self, Multiaddr)> {
        let fun = self.fun;
        self.inner.dial(addr)
            .map_err(move |(dial, addr)| (MapErrListener::new(dial, fun), addr))
    }
}

pub struct MapErrStream<T, F> { stream: T, fun: F }

impl<T, F, E, A, X> Stream for MapErrStream<T, F>
where
    T: Stream<Item = (X, Multiaddr)>,
    X: Future<Error = E>,
    F: FnOnce(E, Multiaddr) -> A + Clone
{
    type Item = (MapErrFuture<X, F>, Multiaddr);
    type Error = T::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self.stream.poll()? {
            Async::Ready(Some((future, addr))) => {
                let f = self.fun.clone();
                let a = addr.clone();
                let future = MapErrFuture {
                    fut: future,
                    args: Some((f, a))
                };
                Ok(Async::Ready(Some((future, addr))))
            }
            Async::Ready(None) => Ok(Async::Ready(None)),
            Async::NotReady => Ok(Async::NotReady)
        }
    }
}

pub struct MapErrFuture<T, F> {
    fut: T,
    args: Option<(F, Multiaddr)>,
}

impl<T, E, F, A> Future for MapErrFuture<T, F>
where
    T: Future<Error = E>,
    F: FnOnce(E, Multiaddr) -> A
{
    type Item = T::Item;
    type Error = A;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.fut.poll() {
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Ok(Async::Ready(x)) => Ok(Async::Ready(x)),
            Err(e) => {
                let (f, a) = self.args.take().expect("Future has not resolved yet");
                Err(f(e, a))
            }
        }
    }
}

