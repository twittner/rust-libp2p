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
pub struct DialerOrElse<T, F> { inner: T, fun: F }

impl<T, F> DialerOrElse<T, F> {
    pub fn new(inner: T, fun: F) -> Self {
        DialerOrElse { inner, fun }
    }
}

impl<D, F, T> Dialer for DialerOrElse<D, F>
where
    D: Dialer,
    F: FnOnce(D::Error, Multiaddr) -> T,
    T: IntoFuture<Item = D::Output>,
{
    type Output = D::Output;
    type Error = T::Error;
    type Outbound = OrElseFuture<D::Outbound, F, T::Future>;

    fn dial(self, addr: Multiaddr) -> Result<Self::Outbound, (Self, Multiaddr)> {
        match self.inner.dial(addr.clone()) {
            Ok(future) => Ok(OrElseFuture {
                inner: Either::A(future),
                args: Some((self.fun, addr))
            }),
            Err((dialer, addr)) => Err((DialerOrElse::new(dialer, self.fun), addr))
        }
    }
}

impl<L, F> Listener for DialerOrElse<L, F>
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
            .map_err(move |(listener, addr)| (DialerOrElse::new(listener, fun), addr))
    }

    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        self.inner.nat_traversal(server, observed)
    }
}

#[derive(Debug, Copy, Clone)]
pub struct ListenerOrElse<T, F> { inner: T, fun: F }

impl<T, F> ListenerOrElse<T, F> {
    pub fn new(inner: T, fun: F) -> Self {
        ListenerOrElse { inner, fun }
    }
}

impl<L, F, T> Listener for ListenerOrElse<L, F>
where
    L: Listener,
    F: FnOnce(L::Error, Multiaddr) -> T + Clone,
    T: IntoFuture<Item = L::Output>
{
    type Output = L::Output;
    type Error = T::Error;
    type Inbound = OrElseStream<L::Inbound, F>;
    type Upgrade = OrElseFuture<L::Upgrade, F, T::Future>;

    fn listen_on(self, addr: Multiaddr) -> Result<(Self::Inbound, Multiaddr), (Self, Multiaddr)> {
        match self.inner.listen_on(addr) {
            Ok((stream, addr)) => Ok((OrElseStream { stream, fun: self.fun }, addr)),
            Err((listener, addr)) => Err((ListenerOrElse::new(listener, self.fun), addr)),
        }
    }

    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        self.inner.nat_traversal(server, observed)
    }
}

impl<D, F> Dialer for ListenerOrElse<D, F>
where
    D: Dialer
{
    type Output = D::Output;
    type Error = D::Error;
    type Outbound = D::Outbound;

    fn dial(self, addr: Multiaddr) -> Result<Self::Outbound, (Self, Multiaddr)> {
        let fun = self.fun;
        self.inner.dial(addr)
            .map_err(move |(dialer, addr)| (ListenerOrElse::new(dialer, fun), addr))
    }
}

pub struct OrElseStream<T, F> { stream: T, fun: F }

impl<T, F, A, B, X> Stream for OrElseStream<T, F>
where
    T: Stream<Item = (X, Multiaddr), Error = std::io::Error>,
    X: Future<Error = A>,
    F: FnOnce(A, Multiaddr) -> B + Clone,
    B: IntoFuture<Item = X::Item>
{
    type Item = (OrElseFuture<X, F, B::Future>, Multiaddr);
    type Error = T::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self.stream.poll()? {
            Async::Ready(Some((future, addr))) => {
                let f = self.fun.clone();
                let a = addr.clone();
                let future = OrElseFuture {
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

pub struct OrElseFuture<T, F, U> {
    inner: Either<T, U>,
    args: Option<(F, Multiaddr)>
}

impl<T, A, F, B> Future for OrElseFuture<T, F, B::Future>
where
    T: Future<Error = A>,
    F: FnOnce(A, Multiaddr) -> B,
    B: IntoFuture<Item = T::Item>
{
    type Item = T::Item;
    type Error = <B::Future as Future>::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let future = match self.inner {
            Either::A(ref mut future) => {
                match future.poll() {
                    Ok(a) => return Ok(a),
                    Err(e) => {
                        let (f, a) = self.args.take().expect("Future has already finished");
                        f(e, a).into_future()
                    }
                }
            }
            Either::B(ref mut future) => return future.poll()
        };
        self.inner = Either::B(future);
        Ok(Async::NotReady)
    }
}

