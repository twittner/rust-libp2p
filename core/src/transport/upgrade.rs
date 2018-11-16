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

use futures::{future::Either, prelude::*};
use multiaddr::Multiaddr;
use crate::{
    transport::{Dialer, Listener, TransportError},
    upgrade::{
        apply_inbound,
        apply_outbound,
        InboundUpgrade,
        InboundUpgradeApply,
        OutboundUpgrade,
        OutboundUpgradeApply
    }
};
use tokio_io::{AsyncRead, AsyncWrite};

#[derive(Debug, Copy, Clone)]
pub struct DialerUpgrade<T, U> { inner: T, upgrade: U }

impl<T, U> DialerUpgrade<T, U> {
    pub fn new(inner: T, upgrade: U) -> Self {
        DialerUpgrade { inner, upgrade }
    }
}

impl<D, U> Dialer for DialerUpgrade<D, U>
where
    D: Dialer,
    D::Output: AsyncRead + AsyncWrite,
    U: OutboundUpgrade<D::Output>
{
    type Output = U::Output;
    type Error = TransportError<D::Error, U::Error>;
    type Outbound = OutboundUpgradeFuture<D::Outbound, U, D::Output>;

    fn dial(self, addr: Multiaddr) -> Result<Self::Outbound, (Self, Multiaddr)> {
        match self.inner.dial(addr.clone()) {
            Ok(outbound) => Ok(OutboundUpgradeFuture {
                future: Either::A(outbound),
                upgrade: Some(self.upgrade)
            }),
            Err((dialer, addr)) => Err((DialerUpgrade::new(dialer, self.upgrade), addr))
        }
    }
}

impl<L, U> Listener for DialerUpgrade<L, U>
where
    L: Listener
{
    type Output = L::Output;
    type Error = L::Error;
    type Inbound = L::Inbound;
    type Upgrade = L::Upgrade;

    fn listen_on(self, addr: Multiaddr) -> Result<(Self::Inbound, Multiaddr), (Self, Multiaddr)> {
        let upgrade = self.upgrade;
        self.inner.listen_on(addr)
            .map_err(move |(listener, addr)| (DialerUpgrade::new(listener, upgrade), addr))
    }

    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        self.inner.nat_traversal(server, observed)
    }
}

#[derive(Debug, Copy, Clone)]
pub struct ListenerUpgrade<T, U> { inner: T, upgrade: U }

impl<T, U> ListenerUpgrade<T, U> {
    pub fn new(inner: T, upgrade: U) -> Self {
        ListenerUpgrade { inner, upgrade }
    }
}

impl<L, U> Listener for ListenerUpgrade<L, U>
where
    L: Listener,
    L::Output: AsyncRead + AsyncWrite,
    U: InboundUpgrade<L::Output> + Clone,
    U::NamesIter: Clone
{
    type Output = U::Output;
    type Error = TransportError<L::Error, U::Error>;
    type Inbound = InboundUpgradeStream<L::Inbound, U>;
    type Upgrade = InboundUpgradeFuture<L::Upgrade, U, L::Output>;

    fn listen_on(self, addr: Multiaddr) -> Result<(Self::Inbound, Multiaddr), (Self, Multiaddr)> {
        let upgrade = self.upgrade;
        match self.inner.listen_on(addr) {
            Ok((stream, addr)) => Ok((InboundUpgradeStream { stream, upgrade}, addr)),
            Err((listener, addr)) => Err((ListenerUpgrade::new(listener, upgrade), addr))
        }
    }

    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        self.inner.nat_traversal(server, observed)
    }
}

impl<D, U> Dialer for ListenerUpgrade<D, U>
where
    D: Dialer
{
    type Output = D::Output;
    type Error = D::Error;
    type Outbound = D::Outbound;

    fn dial(self, addr: Multiaddr) -> Result<Self::Outbound, (Self, Multiaddr)> {
        let upgrade = self.upgrade;
        self.inner.dial(addr)
            .map_err(move |(dialer, addr)| (ListenerUpgrade::new(dialer, upgrade), addr))
    }
}

pub struct OutboundUpgradeFuture<T, U, C>
where
    U: OutboundUpgrade<C>,
    C: AsyncRead + AsyncWrite
{
    future: Either<T, OutboundUpgradeApply<C, U>>,
    upgrade: Option<U>
}

impl<T, U, C> Future for OutboundUpgradeFuture<T, U, C>
where
    T: Future<Item = C>,
    U: OutboundUpgrade<C>,
    C: AsyncRead + AsyncWrite
{
    type Item = U::Output;
    type Error = TransportError<T::Error, U::Error>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let future = match self.future {
            Either::A(ref mut future) => {
                match future.poll() {
                    Ok(Async::Ready(item)) => {
                        let u = self.upgrade.take().expect("Future has not been completed");
                        apply_outbound(item, u)
                    }
                    Ok(Async::NotReady) => return Ok(Async::NotReady),
                    Err(e) => return Err(TransportError::Transport(e))
                }
            }
            Either::B(ref mut future) => {
                match future.poll() {
                    Ok(Async::Ready(item)) => return Ok(Async::Ready(item)),
                    Ok(Async::NotReady) => return Ok(Async::NotReady),
                    Err(e) => return Err(TransportError::Upgrade(e))
                }
            }
        };
        self.future = Either::B(future);
        Ok(Async::NotReady)
    }
}

pub struct InboundUpgradeFuture<T, U, C>
where
    U: InboundUpgrade<C>,
    C: AsyncRead + AsyncWrite
{
    future: Either<T, InboundUpgradeApply<C, U>>,
    upgrade: Option<U>
}

impl<T, U, C> Future for InboundUpgradeFuture<T, U, C>
where
    T: Future<Item = C>,
    U: InboundUpgrade<C>,
    C: AsyncRead + AsyncWrite,
    U::NamesIter: Clone
{
    type Item = U::Output;
    type Error = TransportError<T::Error, U::Error>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let future = match self.future {
            Either::A(ref mut future) => {
                match future.poll() {
                    Ok(Async::Ready(item)) => {
                        let u = self.upgrade.take().expect("Future has not been completed");
                        apply_inbound(item, u)
                    }
                    Ok(Async::NotReady) => return Ok(Async::NotReady),
                    Err(e) => return Err(TransportError::Transport(e))
                }
            }
            Either::B(ref mut future) => {
                match future.poll() {
                    Ok(Async::Ready(item)) => return Ok(Async::Ready(item)),
                    Ok(Async::NotReady) => return Ok(Async::NotReady),
                    Err(e) => return Err(TransportError::Upgrade(e))
                }
            }
        };
        self.future = Either::B(future);
        Ok(Async::NotReady)
    }
}

pub struct InboundUpgradeStream<T, U> { stream: T, upgrade: U }

impl<T, U, F, C> Stream for InboundUpgradeStream<T, U>
where
    T: Stream<Item = (F, Multiaddr), Error = std::io::Error>,
    U: InboundUpgrade<C> + Clone,
    F: Future<Item = C>,
    C: AsyncRead + AsyncWrite
{
    type Item = (InboundUpgradeFuture<F, U, C>, Multiaddr);
    type Error = T::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        let upgrade = self.upgrade.clone();
        match self.stream.poll()? {
            Async::Ready(Some((future, addr))) => {
                let future = InboundUpgradeFuture {
                    future: Either::A(future),
                    upgrade: Some(upgrade)
                };
                Ok(Async::Ready(Some((future, addr))))
            }
            Async::Ready(None) => Ok(Async::Ready(None)),
            Async::NotReady => Ok(Async::NotReady)
        }
    }
}

