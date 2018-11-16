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

//! Handles entering a connection with a peer.
//!
//! The two main elements of this module are the `Transport` and `ConnectionUpgrade` traits.
//! `Transport` is implemented on objects that allow dialing and listening. `ConnectionUpgrade` is
//! implemented on objects that make it possible to upgrade a connection (for example by adding an
//! encryption middleware to the connection).
//!
//! Thanks to the `Transport::or_transport`, `Transport::with_upgrade` and
//! `UpgradedNode::or_upgrade` methods, you can combine multiple transports and/or upgrades
//! together in a complex chain of protocols negotiation.

pub mod and_then;
pub mod denied;
pub mod error;
pub mod map;
pub mod map_err;
pub mod memory;
pub mod or;
pub mod or_else;
pub mod refused;
pub mod upgrade;

use crate::upgrade::{InboundUpgrade, OutboundUpgrade};
use futures::prelude::*;
use multiaddr::Multiaddr;
use self::{
    and_then::{DialerAndThen, ListenerAndThen},
    map::{MapDialer, MapListener},
    map_err::{MapErrListener, MapErrDialer},
    or_else::{DialerOrElse, ListenerOrElse},
    upgrade::{DialerUpgrade, ListenerUpgrade}
};
use tokio_io::{AsyncRead, AsyncWrite};

pub use self::{
    error::TransportError,
    denied::Denied,
    refused::Refused,
    or::Or
};

pub trait Listener {
    type Output;
    type Error;
    type Inbound: Stream<Item = (Self::Upgrade, Multiaddr), Error = std::io::Error>;
    type Upgrade: Future<Item = Self::Output, Error = Self::Error>;

    fn listen_on(self, addr: Multiaddr) -> Result<(Self::Inbound, Multiaddr), (Self, Multiaddr)>
    where
        Self: Sized;

    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr>;
}

pub trait ListenerExt: Listener {
    fn or_listener<T>(self, other: T) -> Or<Self, T>
    where
        Self: Sized
    {
        Or::new(self, other)
    }

    fn map_listener<F, T>(self, f: F) -> MapListener<Self, F>
    where
        Self: Sized,
        F: FnOnce(Self::Output, Multiaddr) -> T + Clone
    {
        MapListener::new(self, f)
    }

    fn map_listener_err<F, E>(self, f: F) -> MapErrListener<Self, F>
    where
        Self: Sized,
        F: FnOnce(Self::Error, Multiaddr) -> E + Clone
    {
        MapErrListener::new(self, f)
    }

    fn listener_and_then<F, T>(self, f: F) -> ListenerAndThen<Self, F>
    where
        Self: Sized,
        F: FnOnce(Self::Output, Multiaddr) -> T + Clone,
        T: IntoFuture<Error = Self::Error>
    {
        ListenerAndThen::new(self, f)
    }

    fn listener_or_else<F, T>(self, f: F) -> ListenerOrElse<Self, F>
    where
        Self: Sized,
        F: FnOnce(Self::Error, Multiaddr) -> T + Clone,
        T: IntoFuture<Item = Self::Output>
    {
        ListenerOrElse::new(self, f)
    }

    fn with_listener_upgrade<U>(self, upgrade: U) -> ListenerUpgrade<Self, U>
    where
        Self: Sized,
        Self::Output: AsyncRead + AsyncWrite,
        U: InboundUpgrade<Self::Output> + Clone
    {
        ListenerUpgrade::new(self, upgrade)
    }
}

impl<L: Listener> ListenerExt for L {}

pub trait Dialer {
    type Output;
    type Error;
    type Outbound: Future<Item = Self::Output, Error = Self::Error>;

    fn dial(self, addr: Multiaddr) -> Result<Self::Outbound, (Self, Multiaddr)>
    where
        Self: Sized;
}

pub trait DialerExt: Dialer {
    fn or_dialer<T>(self, other: T) -> Or<Self, T>
    where
        Self: Sized
    {
        Or::new(self, other)
    }

    fn map_dialer<F, T>(self, f: F) -> MapDialer<Self, F>
    where
        Self: Sized,
        F: FnOnce(Self::Output, Multiaddr) -> T
    {
        MapDialer::new(self, f)
    }

    fn map_dialer_err<F, E>(self, f: F) -> MapErrDialer<Self, F>
    where
        Self: Sized,
        F: FnOnce(Self::Error, Multiaddr) -> E
    {
        MapErrDialer::new(self, f)
    }

    fn dialer_and_then<F, T>(self, f: F) -> DialerAndThen<Self, F>
    where
        Self: Sized,
        F: FnOnce(Self::Output, Multiaddr) -> T,
        T: IntoFuture<Error = Self::Error>,
    {
        DialerAndThen::new(self, f)
    }

    fn dialer_or_else<F, T>(self, f: F) -> DialerOrElse<Self, F>
    where
        Self: Sized,
        F: FnOnce(Self::Error, Multiaddr) -> T,
        T: IntoFuture<Item = Self::Output>
    {
        DialerOrElse::new(self, f)
    }

    fn with_dialer_upgrade<U>(self, upgrade: U) -> DialerUpgrade<Self, U>
    where
        Self: Sized,
        Self::Output: AsyncRead + AsyncWrite,
        U: OutboundUpgrade<Self::Output>
    {
        DialerUpgrade::new(self, upgrade)
    }
}

impl<D: Dialer> DialerExt for D {}

