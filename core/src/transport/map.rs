// Copyright 2017-2018 Parity Technologies (UK) Ltd.
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
pub struct MapDialer<T, F> { inner: T, fun: F }

impl<T, F> MapDialer<T, F> {
    pub fn new(inner: T, fun: F) -> Self {
        MapDialer { inner, fun }
    }
}

impl<D, F, T> Dialer for MapDialer<D, F>
where
    D: Dialer,
    F: FnOnce(D::Output, Multiaddr) -> T
{
    type Output = T;
    type Error = D::Error;
    type Outbound = MapFuture<D::Outbound, F>;

    fn dial(self, addr: Multiaddr) -> Result<Self::Outbound, (Self, Multiaddr)> {
        let fun = self.fun;
        match self.inner.dial(addr.clone()) {
            Ok(future) => Ok(MapFuture {
                inner: future,
                args: Some((fun, addr))
            }),
            Err((dialer, addr)) => Err((MapDialer::new(dialer, fun), addr))
        }
    }
}

impl<L, F> Listener for MapDialer<L, F>
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
            .map_err(move |(listener, addr)| (MapDialer::new(listener, fun), addr))
    }

    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        self.inner.nat_traversal(server, observed)
    }
}

#[derive(Debug, Copy, Clone)]
pub struct MapListener<T, F> { inner: T, fun: F }

impl<T, F> MapListener<T, F> {
    pub fn new(inner: T, fun: F) -> Self {
        MapListener { inner, fun }
    }
}

impl<L, F, T> Listener for MapListener<L, F>
where
    L: Listener,
    F: FnOnce(L::Output, Multiaddr) -> T + Clone,
{
    type Output = T;
    type Error = L::Error;
    type Inbound = MapStream<L::Inbound, F>;
    type Upgrade = MapFuture<L::Upgrade, F>;

    fn listen_on(self, addr: Multiaddr) -> Result<(Self::Inbound, Multiaddr), (Self, Multiaddr)> {
        match self.inner.listen_on(addr) {
            Ok((stream, addr)) => {
                let stream = MapStream { stream, fun: self.fun };
                Ok((stream, addr))
            }
            Err((listener, addr)) => Err((MapListener::new(listener, self.fun), addr))
        }
    }

    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        self.inner.nat_traversal(server, observed)
    }
}

impl<D, F> Dialer for MapListener<D, F>
where
    D: Dialer
{
    type Output = D::Output;
    type Error = D::Error;
    type Outbound = D::Outbound;

    fn dial(self, addr: Multiaddr) -> Result<Self::Outbound, (Self, Multiaddr)> {
        let fun = self.fun;
        self.inner.dial(addr)
            .map_err(move |(dialer, addr)| (MapListener::new(dialer, fun), addr))
    }
}

pub struct MapStream<T, F> { stream: T, fun: F }

impl<T, F, A, B, X> Stream for MapStream<T, F>
where
    T: Stream<Item = (X, Multiaddr)>,
    X: Future<Item = A>,
    F: FnOnce(A, Multiaddr) -> B + Clone
{
    type Item = (MapFuture<X, F>, Multiaddr);
    type Error = T::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self.stream.poll()? {
            Async::Ready(Some((future, addr))) => {
                let f = self.fun.clone();
                let a = addr.clone();
                let future = MapFuture {
                    inner: future,
                    args: Some((f, a))
                };
                Ok(Async::Ready(Some((future, addr))))
            }
            Async::Ready(None) => Ok(Async::Ready(None)),
            Async::NotReady => Ok(Async::NotReady)
        }
    }
}

pub struct MapFuture<T, F> {
    inner: T,
    args: Option<(F, Multiaddr)>
}

impl<T, A, F, B> Future for MapFuture<T, F>
where
    T: Future<Item = A>,
    F: FnOnce(A, Multiaddr) -> B
{
    type Item = B;
    type Error = T::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let item = try_ready!(self.inner.poll());
        let (f, a) = self.args.take().expect("Future has already finished");
        Ok(Async::Ready(f(item, a)))
    }
}

