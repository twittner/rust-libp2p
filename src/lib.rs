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

//! Libp2p is a peer-to-peer framework.
//!
//! # Major libp2p concepts
//!
//! Here is a list of all the major concepts of libp2p.
//!
//! ## Multiaddr
//!
//! A `Multiaddr` is a way to reach a node. Examples:
//!
//! * `/ip4/80.123.90.4/tcp/5432`
//! * `/ip6/[::1]/udp/10560`
//! * `/unix//path/to/socket`
//!
//! ## Transport
//!
//! `Transport` is a trait that represents an object capable of dialing multiaddresses or
//! listening on multiaddresses. The `Transport` produces an output which varies depending on the
//! object that implements the trait.
//!
//! Each implementation of `Transport` typically supports only some multiaddresses. For example
//! the `TcpDialer` type (which implements `Dialer`) only supports multiaddresses of the format
//! `/ip4/.../tcp/...`.
//!
//! Example:
//!
//! ```rust
//! use libp2p::{Multiaddr, Dialer, tcp::TcpDialer};
//! let tcp_dialer = TcpDialer::default();
//! let addr: Multiaddr = "/ip4/98.97.96.95/tcp/20500".parse().expect("invalid multiaddr");
//! let _outgoing_connec = tcp_dialer.dial(addr);
//! // Note that `_outgoing_connec` is a `Future`, and therefore doesn't do anything by itself
//! // unless it is run through a tokio runtime.
//! ```
//!
//! The easiest way to create a transport is to use the `CommonTransport` struct. This struct
//! provides support for the most common protocols.
//!
//! Example:
//!
//! ```rust
//! use libp2p::CommonTransport;
//! let _transport = CommonTransport::new();
//! // _transport.dial(...);
//! ```
//!
//! See the documentation of the `libp2p-core` crate for more details about transports.
//!
//! # Connection upgrades
//!
//! Once a connection has been opened with a remote through a `Transport`, it can be *upgraded*.
//! This consists in negotiating a protocol with the remote (through the `multistream-select`
//! protocol), and applying that protocol on the socket.
//!
//! Example upgrades:
//!
//! - Adding a security layer on top of the connection.
//! - Applying multiplexing, so that a connection can be split into multiple substreams.
//! - Negotiating a specific protocol, such as *ping* or *kademlia*.
//!
//! A potential connection upgrade is represented with the `ConnectionUpgrade` trait. The trait
//! consists in a protocol name plus a method that turns the socket into an `Output` object whose
//! nature and type is specific to each upgrade. For example, if you upgrade a connection with a
//! security layer, the output might contain an encrypted stream and the public key of the remote.
//!
//! You can combine a `Transport` with a compatible `ConnectionUpgrade` in order to obtain another
//! `Transport` that yields the output of the upgrade.
//!
//! Example:
//!
//! ```rust
//! # #[cfg(all(not(target_os = "emscripten"), feature = "libp2p-secio"))] {
//! use libp2p::{Dialer, DialerExt, tcp::TcpDialer, secio::{SecioConfig, SecioKeyPair}};
//! let tcp_dialer = TcpDialer::default();
//! let secio_upgrade = SecioConfig::new(SecioKeyPair::ed25519_generated().unwrap());
//! let with_security = tcp_dialer.with_upgrade(secio_upgrade);
//! // let _ = with_security.dial(...);
//! // `with_security` also implements the `Transport` trait, and all the connections opened
//! // through it will automatically negotiate the `secio` protocol.
//! # }
//! ```
//!
//! See the documentation of the `libp2p-core` crate for more details about upgrades.
//!
//! ## Swarm
//!
//! Once you have created an object that implements the `Transport` trait, you can put it in a
//! *swarm*. This is done by calling the `swarm()` freestanding function with the transport
//! alongside with a function or a closure that will turn the output of the upgrade (usually an
//! actual protocol, as explained above) into a `Future` producing `()`.
//!
//! See the documentation of the `libp2p-core` crate for more details about creating a swarm.
//!
//! # Using libp2p
//!
//! This section contains details about how to use libp2p in practice.
//!
//! The most simple way to use libp2p consists in the following steps:
//!
//! - Create a *base* implementation of `Transport` that combines all the protocols you want and
//!   the upgrades you want, such as the security layer and multiplexing.
//! - Create structs that implement the `ConnectionUpgrade` trait for the protocols you want to
//!   create, or use the protocols provided by the `libp2p` crate.
//! - Create a swarm that combines your base transport and all the upgrades and that handles the
//!   behaviour that happens.
//! - Use this swarm to dial and listen.
//!
//! You probably also want to have some sort of nodes discovery mechanism, so that you
//! automatically connect to nodes of the network. The details of this haven't been fleshed out
//! in libp2p and will be written later.
//!

pub extern crate bytes;
pub extern crate futures;
pub extern crate multiaddr;
pub extern crate multihash;
pub extern crate tokio_io;
pub extern crate tokio_codec;

extern crate tokio_executor;

pub extern crate libp2p_core as core;
#[cfg(not(target_os = "emscripten"))]
pub extern crate libp2p_dns as dns;
pub extern crate libp2p_identify as identify;
pub extern crate libp2p_kad as kad;
pub extern crate libp2p_floodsub as floodsub;
pub extern crate libp2p_mplex as mplex;
pub extern crate libp2p_peerstore as peerstore;
pub extern crate libp2p_ping as ping;
pub extern crate libp2p_ratelimit as ratelimit;
pub extern crate libp2p_relay as relay;
pub extern crate libp2p_secio as secio;
#[cfg(not(target_os = "emscripten"))]
pub extern crate libp2p_tcp_transport as tcp;
pub extern crate libp2p_transport_timeout as transport_timeout;
pub extern crate libp2p_uds as uds;
#[cfg(feature = "libp2p-websocket")]
pub extern crate libp2p_websocket as websocket;
pub extern crate libp2p_yamux as yamux;

mod transport_ext;

pub mod simple;

pub use self::core::{Dialer, DialerExt, Listener, ListenerExt, Transport, PeerId, transport};
pub use self::multiaddr::Multiaddr;
pub use self::simple::SimpleProtocol;
pub use self::transport_ext::{DialerExtra, ListenerExtra};
pub use self::transport_timeout::TransportTimeout;

/// Implementation of `Transport` that supports the most common protocols.
///
/// The list currently is TCP/IP, DNS, and WebSockets. However this list could change in the
/// future to get new transports.
#[derive(Debug, Clone)]
pub struct CommonTransport {
    // The actual implementation of everything.
    inner: CommonTransportInner
}

#[cfg(all(not(target_os = "emscripten"), feature = "libp2p-websocket"))]
pub type InnerImplementation =
    Transport<
        transport::Or<
            dns::DnsDialer<tcp::TcpDialer>,
            websocket::WsDialer<dns::DnsDialer<tcp::TcpDialer>>
        >,
        transport::Or<
            tcp::TcpListener,
            websocket::WsListener<tcp::TcpListener>
        >
    >;

#[cfg(all(not(target_os = "emscripten"), not(feature = "libp2p-websocket")))]
pub type InnerImplementation = Transport<dns::DnsDialer<tcp::TcpDialer>, tcp::TcpListener>;

#[cfg(target_os = "emscripten")]
pub type InnerImplementation = Transport<websocket::BrowserWsDialer, transport::DeniedListener>;

#[derive(Debug, Clone)]
struct CommonTransportInner {
    inner: InnerImplementation,
}

impl CommonTransport {
    /// Initializes the `CommonTransport`.
    #[inline]
    #[cfg(not(target_os = "emscripten"))]
    pub fn new() -> CommonTransport {
        let dialer = dns::DnsDialer::new(tcp::TcpDialer::default());

        #[cfg(feature = "libp2p-websocket")]
        let dialer = {
            let dns_dialer = dialer.clone();
            dialer.or(websocket::WsDialer::new(dns_dialer))
        };

        let listener = tcp::TcpListener::default();

        #[cfg(feature = "libp2p-websocket")]
        let listener = {
            let tcp_listener = listener.clone();
            listener.or(websocket::WsListener::new(tcp_listener))
        };

        CommonTransport {
            inner: CommonTransportInner { inner: Transport::new(dialer, listener) }
        }
    }

    /// Initializes the `CommonTransport`.
    #[inline]
    #[cfg(target_os = "emscripten")]
    pub fn new() -> CommonTransport {
        let inner = Transport::dialer(websocket::BrowserWsDialer::new());
        CommonTransport {
            inner: CommonTransportInner { inner: inner }
        }
    }
}

impl Dialer for CommonTransport {
    type Output = <InnerImplementation as Dialer>::Output;
    type Error = <InnerImplementation as Dialer>::Error;
    type Outbound = <InnerImplementation as Dialer>::Outbound;

    fn dial(self, addr: Multiaddr) -> Result<Self::Outbound, (Self, Multiaddr)> {
        self.inner.inner.dial(addr).or_else(|(inner, addr)| {
            let trans = CommonTransport { inner: CommonTransportInner { inner } };
            Err((trans, addr))
        })
    }
}

impl Listener for CommonTransport {
    type Output = <InnerImplementation as Listener>::Output;
    type Error = <InnerImplementation as Listener>::Error;
    type Inbound = <InnerImplementation as Listener>::Inbound;
    type Upgrade = <InnerImplementation as Listener>::Upgrade;

    fn listen_on(self, addr: Multiaddr) -> Result<(Self::Inbound, Multiaddr), (Self, Multiaddr)> {
        self.inner.inner.listen_on(addr).or_else(|(inner, addr)| {
            let trans = CommonTransport { inner: CommonTransportInner { inner } };
            Err((trans, addr))
        })
    }

    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        self.inner.inner.nat_traversal(server, observed)
    }
}

/// The `multiaddr!` macro is an easy way for a user to create a `Multiaddr`.
///
/// Example:
///
/// ```rust
/// # #[macro_use]
/// # extern crate libp2p;
/// # fn main() {
/// let _addr = multiaddr![Ip4([127, 0, 0, 1]), Tcp(10500u16)];
/// # }
/// ```
///
/// Each element passed to `multiaddr![]` should be a variant of the `Protocol` enum. The
/// optional parameter is casted into the proper type with the `Into` trait.
///
/// For example, `Ip4([127, 0, 0, 1])` works because `Ipv4Addr` implements `From<[u8; 4]>`.
#[macro_export]
macro_rules! multiaddr {
    ($($comp:ident $(($param:expr))*),+) => {
        {
            use std::iter;
            let elem = iter::empty::<$crate::multiaddr::Protocol>();
            $(
                let elem = {
                    let cmp = $crate::multiaddr::Protocol::$comp $(( $param.into() ))*;
                    elem.chain(iter::once(cmp))
                };
            )+
            elem.collect::<$crate::Multiaddr>()
        }
    }
}
