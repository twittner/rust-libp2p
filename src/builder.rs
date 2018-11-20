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

use core::prelude::*;

#[derive(Debug, Clone)]
pub struct Transport<D, L> { dialer: D, listener: L }

impl<D, L> Transport<D, L> {
    pub fn into(self) -> (D, L) {
        (self.dialer, self.listener)
    }

    pub fn replace_dialer<F, T>(self, f: F) -> Transport<T, L>
    where
        F: FnOnce(D) -> T
    {
        Transport {
            dialer: f(self.dialer),
            listener: self.listener
        }
    }

    pub fn replace_listener<F, T>(self, f: F) -> Transport<D, T>
    where
        F: FnOnce(L) -> T
    {
        Transport {
            dialer: self.dialer,
            listener: f(self.listener)
        }
    }
}

impl Transport<Denied, Denied> {
    pub fn denied() -> Self {
        Transport { dialer: Denied, listener: Denied}
    }
}

impl<DT, DE, LT, LE> Transport<Refused<DT, DE>, Refused<LT, LE>> {
    pub fn refused() -> Self {
        Transport { dialer: Refused::new(), listener: Refused::new() }
    }
}

impl<D, L> Transport<D, L>
where
    D: Dialer,
    L: Listener
{
    pub fn new(dialer: D, listener: L) -> Self {
        Transport { dialer, listener }
    }
}
impl<L> Transport<Denied, L>
where
    L: Listener
{
    pub fn listener(listener: L) -> Self {
        Transport::new(Denied, listener)
    }

    pub fn with_dialer<D>(self, dialer: D) -> Transport<D, L>
    where
        D: Dialer
    {
        Transport { dialer, listener: self.listener }
    }
}

impl<D> Transport<D, Denied>
where
    D: Dialer
{
    pub fn dialer(dialer: D) -> Self {
        Transport::new(dialer, Denied)
    }

    pub fn with_listener<L>(self, listener: L) -> Transport<D, L>
    where
        L: Listener
    {
        Transport { dialer: self.dialer, listener }
    }
}

impl<D, L> Dialer for Transport<D, L>
where
    D: Dialer
{
    type Output = D::Output;
    type Error = D::Error;
    type Outbound = D::Outbound;

    fn dial(self, addr: Multiaddr) -> Result<Self::Outbound, (Self, Multiaddr)> {
        let dialer = self.dialer;
        let listener = self.listener;
        dialer.dial(addr)
            .map_err(move |(dialer, addr)| (Transport { dialer, listener }, addr))
    }
}

impl<D, L> Listener for Transport<D, L>
where
    L: Listener
{
    type Output = L::Output;
    type Error = L::Error;
    type Inbound = L::Inbound;
    type Upgrade = L::Upgrade;

    fn listen_on(self, addr: Multiaddr) -> Result<(Self::Inbound, Multiaddr), (Self, Multiaddr)> {
        let dialer = self.dialer;
        let listener = self.listener;
        listener.listen_on(addr)
            .map_err(move |(listener, addr)| (Transport { dialer, listener }, addr))
    }

    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        self.listener.nat_traversal(server, observed)
    }
}

