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

//! A basic chat application demonstrating libp2p and the mDNS and floodsub protocols.
//!
//! Using two terminal windows, start two instances. If you local network allows mDNS,
//! they will automatically connect. Type a message in either terminal and hit return: the
//! message is sent and printed in the other terminal. Close with Ctrl-c.
//!
//! You can of course open more terminal windows and add more participants.
//! Dialing any of the other peers will propagate the new participant to all
//! chat members and everyone will receive all messages.
//!
//! # If they don't automatically connect
//!
//! If the nodes don't automatically connect, take note of the listening address of the first
//! instance and start the second with this address as the first argument. In the first terminal
//! window, run:
//!
//! ```sh
//! cargo run --example chat
//! ```
//!
//! It will print the PeerId and the listening address, e.g. `Listening on
//! "/ip4/0.0.0.0/tcp/24915"`
//!
//! In the second terminal window, start a new instance of the example with:
//!
//! ```sh
//! cargo run --example chat -- /ip4/127.0.0.1/tcp/24915
//! ```
//!
//! The two nodes then connect.

extern crate env_logger;
extern crate futures;
extern crate libp2p;
extern crate openssl;
extern crate quicli;
extern crate structopt;
extern crate tokio;
extern crate void;

use futures::prelude::*;
use libp2p::{NetworkBehaviour, core::PublicKey, tokio_codec::{FramedRead, LinesCodec}};
use openssl::{ec::{EcGroup, EcKey}, nid::Nid, pkey::PKey};
use quicli::prelude::*;
use structopt::StructOpt;
use std::{fs::File, io::{self, Write}};

#[derive(Debug, StructOpt)]
enum Cli {
    #[structopt(name = "gen")]
    Gen {
        #[structopt(long = "out", short = "o")]
        out: String
    },
    #[structopt(name = "run")]
    Run {
        #[structopt(long = "key", short = "k")]
        key: String,

        #[structopt(long = "dial", short = "d")]
        dial: Option<String>
    }
}

fn main() -> CliResult {
    env_logger::init();

    let (key, dial) = match Cli::from_args() {
        Cli::Gen { out }=> {
            let g = EcGroup::from_curve_name(Nid::ED25519)?;
            let k = EcKey::generate(&g)?.private_key_to_pem()?;
            let mut f = File::create(out)?;
            f.write_all(&k)?;
            return Ok(())
        }
        Cli::Run { key, dial } => (key, dial)
    };

    let private_key = PKey::private_key_from_pem(read_file(&key)?.as_bytes())?;
    let public_key = PublicKey::Ed25519(private_key.public_key_to_der()?);

    let rt = tokio::runtime::Runtime::new()?;

    // Set up a an encrypted DNS-enabled TCP Transport over the Mplex and Yamux protocols
    let transport = libp2p_quic::QuicConfig::new(rt.executor(), private_key.ec_key()?)?;

    // Create a Floodsub topic
    let floodsub_topic = libp2p::floodsub::TopicBuilder::new("chat").build();

    // We create a custom network behaviour that combines floodsub and mDNS.
    // In the future, we want to improve libp2p to make this easier to do.
    #[derive(NetworkBehaviour)]
    struct MyBehaviour<TSubstream: libp2p::tokio_io::AsyncRead + libp2p::tokio_io::AsyncWrite> {
        floodsub: libp2p::floodsub::Floodsub<TSubstream>,
        mdns: libp2p::mdns::Mdns<TSubstream>,
    }

    impl<TSubstream: libp2p::tokio_io::AsyncRead + libp2p::tokio_io::AsyncWrite> libp2p::core::swarm::NetworkBehaviourEventProcess<void::Void> for MyBehaviour<TSubstream> {
        fn inject_event(&mut self, _ev: void::Void) {
            void::unreachable(_ev)
        }
    }

    impl<TSubstream: libp2p::tokio_io::AsyncRead + libp2p::tokio_io::AsyncWrite> libp2p::core::swarm::NetworkBehaviourEventProcess<libp2p::floodsub::FloodsubEvent> for MyBehaviour<TSubstream> {
        // Called when `floodsub` produces an event.
        fn inject_event(&mut self, message: libp2p::floodsub::FloodsubEvent) {
            if let libp2p::floodsub::FloodsubEvent::Message(message) = message {
                println!("Received: '{:?}' from {:?}", String::from_utf8_lossy(&message.data), message.source);
            }
        }
    }

    // Create a Swarm to manage peers and events
    let mut swarm = {
        let mut behaviour = MyBehaviour {
            floodsub: libp2p::floodsub::Floodsub::new(public_key.clone().into_peer_id()),
            mdns: libp2p::mdns::Mdns::new()?
        };

        behaviour.floodsub.subscribe(floodsub_topic.clone());
        libp2p::Swarm::new(transport, behaviour, libp2p::core::topology::MemoryTopology::empty(public_key))
    };

    // Listen on all interfaces and whatever port the OS assigns
    let addr = libp2p::Swarm::listen_on(&mut swarm, "/ip4/127.0.0.1/udp/0/quic".parse()?)?;
    println!("Listening on {:?}", addr);

    // Reach out to another node if specified
    if let Some(to_dial) = dial {
        let dialing = to_dial.clone();
        match to_dial.parse() {
            Ok(to_dial) => {
                match libp2p::Swarm::dial_addr(&mut swarm, to_dial) {
                    Ok(_) => println!("Dialed {:?}", dialing),
                    Err(e) => println!("Dial {:?} failed: {:?}", dialing, e)
                }
            },
            Err(err) => println!("Failed to parse address to dial: {:?}", err),
        }
    }

    // Read full lines from stdin
    let stdin = tokio_stdin_stdout::stdin(0);
    let mut framed_stdin = FramedRead::new(stdin, LinesCodec::new());

    // Kick it off
    rt.block_on_all(futures::future::poll_fn(move || {
        loop {
            match framed_stdin.poll()? {
                Async::Ready(Some(line)) => swarm.floodsub.publish(&floodsub_topic, line.as_bytes()),
                Async::Ready(None) => return Err(io::Error::new(io::ErrorKind::Other, "stdin closed")),
                Async::NotReady => break,
            };
        }

        loop {
            match swarm.poll()? {
                Async::Ready(Some(_)) => {}
                Async::Ready(None) | Async::NotReady => break
            }
        }

        Ok(Async::NotReady)
    }))?;

    Ok(())
}

