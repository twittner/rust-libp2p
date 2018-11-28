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
use std::fmt;
use std::io::Error as IoError;
use std::sync::Arc;
use transport::{MultiaddrSeq, Transport};

/// See the `Transport::boxed` method.
#[inline]
pub fn boxed<T>(transport: T) -> Boxed<T::Output>
where
    T: Transport + Clone + Send + Sync + 'static,
    T::Dial: Send + 'static,
    T::ListenOn: Send + 'static,
    T::Listener: Send + 'static,
    T::ListenerUpgrade: Send + 'static,
{
    Boxed {
        inner: Arc::new(transport) as Arc<_>,
    }
}

pub type Dial<O> = Box<Future<Item = O, Error = IoError> + Send>;
pub type ListenOn<O> = Box<Future<Item = (Listener<O>, MultiaddrSeq), Error = IoError> + Send>;
pub type Listener<O> = Box<Stream<Item = (ListenerUpgrade<O>, Multiaddr), Error = IoError> + Send>;
pub type ListenerUpgrade<O> = Box<Future<Item = O, Error = IoError> + Send>;
pub type Incoming<O> = Box<Future<Item = (IncomingUpgrade<O>, Multiaddr), Error = IoError> + Send>;
pub type IncomingUpgrade<O> = Box<Future<Item = O, Error = IoError> + Send>;

trait Abstract<O> {
    fn listen_on(&self, addr: Multiaddr) -> Result<ListenOn<O>, Multiaddr>;
    fn dial(&self, addr: Multiaddr) -> Result<Dial<O>, Multiaddr>;
    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr>;
}

impl<T, O> Abstract<O> for T
where
    T: Transport<Output = O> + Clone + 'static,
    T::Dial: Send + 'static,
    T::ListenOn: Send + 'static,
    T::Listener: Send + 'static,
    T::ListenerUpgrade: Send + 'static,
{
    fn listen_on(&self, addr: Multiaddr) -> Result<ListenOn<O>, Multiaddr> {
        let listen_on = Transport::listen_on(self.clone(), addr).map_err(|(_, addr)| addr)?;
        let fut = listen_on
            .map(|(listener, addrs)| {
                let listener = listener.map(|(upgrade, addr)| {
                    (Box::new(upgrade) as ListenerUpgrade<O>, addr)
                });
                (Box::new(listener) as Listener<O>, addrs)
            });
        Ok(Box::new(fut) as Box<_>)
    }

    fn dial(&self, addr: Multiaddr) -> Result<Dial<O>, Multiaddr> {
        let fut = Transport::dial(self.clone(), addr)
            .map_err(|(_, addr)| addr)?;
        Ok(Box::new(fut) as Box<_>)
    }

    #[inline]
    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        Transport::nat_traversal(self, server, observed)
    }
}

/// See the `Transport::boxed` method.
pub struct Boxed<O> {
    inner: Arc<Abstract<O> + Send + Sync>,
}

impl<O> fmt::Debug for Boxed<O> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "BoxedTransport")
    }
}

impl<O> Clone for Boxed<O> {
    #[inline]
    fn clone(&self) -> Self {
        Boxed {
            inner: self.inner.clone(),
        }
    }
}

impl<O> Transport for Boxed<O> {
    type Output = O;
    type ListenOn = ListenOn<O>;
    type Listener = Listener<O>;
    type ListenerUpgrade = ListenerUpgrade<O>;
    type Dial = Dial<O>;

    #[inline]
    fn listen_on(self, addr: Multiaddr) -> Result<Self::ListenOn, (Self, Multiaddr)> {
        match self.inner.listen_on(addr) {
            Ok(listen) => Ok(listen),
            Err(addr) => Err((self, addr)),
        }
    }

    #[inline]
    fn dial(self, addr: Multiaddr) -> Result<Self::Dial, (Self, Multiaddr)> {
        match self.inner.dial(addr) {
            Ok(dial) => Ok(dial),
            Err(addr) => Err((self, addr)),
        }
    }

    #[inline]
    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        self.inner.nat_traversal(server, observed)
    }
}
