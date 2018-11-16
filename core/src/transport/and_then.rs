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

use futures::{future::Either, prelude::*};
use multiaddr::Multiaddr;
use crate::transport::{Dialer, Listener};

#[derive(Debug, Copy, Clone)]
pub struct ListenerAndThen<T, F> { inner: T, fun: F }

impl<T, F> ListenerAndThen<T, F> {
    pub fn new(inner: T, fun: F) -> Self {
        ListenerAndThen { inner, fun }
    }
}

impl<D, F> Dialer for ListenerAndThen<D, F>
where
    D: Dialer
{
    type Output = D::Output;
    type Error = D::Error;
    type Outbound = D::Outbound;

    fn dial(self, addr: Multiaddr) -> Result<Self::Outbound, (Self, Multiaddr)> {
        let fun = self.fun;
        self.inner.dial(addr)
            .map_err(move |(dialer, addr)| (ListenerAndThen::new(dialer, fun), addr))
    }
}

impl<L, F, T> Listener for ListenerAndThen<L, F>
where
    L: Listener,
    F: FnOnce(L::Output, Multiaddr) -> T + Clone,
    T: IntoFuture<Error = L::Error>
{
    type Output = T::Item;
    type Error = L::Error;
    type Inbound = AndThenStream<L::Inbound, F>;
    type Upgrade = AndThenFuture<L::Upgrade, F, T::Future>;

    fn listen_on(self, addr: Multiaddr) -> Result<(Self::Inbound, Multiaddr), (Self, Multiaddr)> {
        match self.inner.listen_on(addr) {
            Ok((stream, addr)) => Ok((AndThenStream { stream, fun: self.fun }, addr)),
            Err((listener, addr)) => Err((ListenerAndThen::new(listener, self.fun), addr)),
        }
    }

    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        self.inner.nat_traversal(server, observed)
    }
}

#[derive(Debug, Copy, Clone)]
pub struct DialerAndThen<T, F> { inner: T, fun: F }

impl<T, F> DialerAndThen<T, F> {
    pub fn new(inner: T, fun: F) -> Self {
        DialerAndThen { inner, fun }
    }
}

impl<D, F, T> Dialer for DialerAndThen<D, F>
where
    D: Dialer,
    F: FnOnce(D::Output, Multiaddr) -> T,
    T: IntoFuture<Error = D::Error>
{
    type Output = T::Item;
    type Error = D::Error;
    type Outbound = AndThenFuture<D::Outbound, F, T::Future>;

    fn dial(self, addr: Multiaddr) -> Result<Self::Outbound, (Self, Multiaddr)> {
        let fun = self.fun;
        match self.inner.dial(addr.clone()) {
            Ok(future) => Ok(AndThenFuture {
                inner: Either::A(future),
                args: Some((fun, addr))
            }),
            Err((dialer, addr)) => Err((DialerAndThen::new(dialer, fun), addr))
        }
    }
}

impl<L, F> Listener for DialerAndThen<L, F>
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
            .map_err(move |(listener, addr)| (DialerAndThen::new(listener, fun), addr))
    }

    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        self.inner.nat_traversal(server, observed)
    }
}

pub struct AndThenStream<T, F> { stream: T, fun: F }

impl<T, F, A, B, X> Stream for AndThenStream<T, F>
where
    T: Stream<Item = (X, Multiaddr), Error = std::io::Error>,
    X: Future<Item = A>,
    F: FnOnce(A, Multiaddr) -> B + Clone,
    B: IntoFuture<Error = X::Error>
{
    type Item = (AndThenFuture<X, F, B::Future>, Multiaddr);
    type Error = T::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self.stream.poll()? {
            Async::Ready(Some((future, addr))) => {
                let f = self.fun.clone();
                let a = addr.clone();
                let future = AndThenFuture {
                    inner: Either::A(future),
                    args: Some((f, a))
                };
                Ok(Async::Ready(Some((future, addr))))
            }
            Async::Ready(None) => Ok(Async::Ready(None)),
            Async::NotReady => Ok(Async::NotReady)
        }
    }
}

pub struct AndThenFuture<T, F, U> {
    inner: Either<T, U>,
    args: Option<(F, Multiaddr)>
}

impl<T, A, F, B> Future for AndThenFuture<T, F, B::Future>
where
    T: Future<Item = A>,
    F: FnOnce(A, Multiaddr) -> B,
    B: IntoFuture<Error = T::Error>
{
    type Item = <B::Future as Future>::Item;
    type Error = T::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let future = match self.inner {
            Either::A(ref mut future) => {
                let item = try_ready!(future.poll());
                let (f, a) = self.args.take().expect("Future has already finished");
                f(item, a).into_future()
            }
            Either::B(ref mut future) => return future.poll()
        };
        self.inner = Either::B(future);
        Ok(Async::NotReady)
    }
}

