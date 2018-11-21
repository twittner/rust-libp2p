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

//! A basic chat application demonstrating libp2p and the Floodsub protocol.
//!
//! Using two terminal windows, start two instances. Take note of the listening
//! address of the first instance and start the second with this address as the
//! first argument. In the first terminal window, run:
//! ```text
//! cargo run --example chat
//! ```
//! It will print the PeerId and the listening address, e.g. `Listening on
//! "/ip4/0.0.0.0/tcp/24915"`
//!
//! In the second terminal window, start a new instance of the example with:
//! ```text
//! cargo run --example chat -- /ip4/127.0.0.1/tcp/24915
//! ```
//! The two nodes connect. Type a message in either terminal and hit return: the
//! message is sent and printed in the other terminal.Close with Ctrl-c.
//!
//! You can of course open more terminal windows and add more participants.
//! Dialing any of the other peers will propagate the new participant to all
//! chat members and everyone will receive all messages.

extern crate futures;
extern crate libp2p;
extern crate tokio;

use futures::prelude::*;
use libp2p::{core::prelude::*, secio, mplex, tokio_codec::{FramedRead, LinesCodec}};

fn main() {
    // Create a random PeerId
    let local_key = secio::SecioKeyPair::ed25519_generated().unwrap();
    let local_peer_id = local_key.to_peer_id();
    println!("Local peer id: {:?}", local_peer_id);

    // Set up a an encrypted DNS-enabled TCP Transport over the Mplex protocol
    let transport = libp2p::common_transport()
        .with_listener_upgrade(secio::SecioConfig::new(local_key.clone()))
        .with_dialer_upgrade(secio::SecioConfig::new(local_key))
        .map_listener_err(|e, _| TransportError::Transport(e))
        .map_dialer_err(|e, _| TransportError::Transport(e))
        .and_then(move |out, end, _addr| {
            let peer_id1 = out.remote_key.into_peer_id();
            let peer_id2 = peer_id1.clone();
            let upgrade = mplex::MplexConfig::new()
                .map_inbound(move |muxer| (peer_id1, muxer))
                .map_outbound(move |muxer| (peer_id2, muxer));
            upgrade::apply(out.stream, upgrade, end).map_err(TransportError::Upgrade)
        })
        .map_listener_err(|e, _| e.into_io_error())
        .map_dialer_err(|e, _| e.into_io_error());

    // Create a Floodsub topic
    let floodsub_topic = libp2p::floodsub::TopicBuilder::new("chat").build();

    // Create a Swarm to manage peers and events
    let mut swarm = {
        let mut behaviour = libp2p::floodsub::FloodsubBehaviour::new(local_peer_id);
        behaviour.subscribe(floodsub_topic.clone());
        libp2p::Swarm::new(transport, behaviour, libp2p::core::topology::MemoryTopology::empty())
    };

    // Listen on all interfaces and whatever port the OS assigns
    let addr = libp2p::Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/tcp/0".parse().unwrap()).unwrap();
    println!("Listening on {:?}", addr);

    // Reach out to another node
    if let Some(to_dial) = std::env::args().nth(1) {
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
    tokio::run(futures::future::poll_fn(move || -> Result<_, ()> {
        loop {
            match framed_stdin.poll().expect("Error while polling stdin") {
                Async::Ready(Some(line)) => swarm.publish(&floodsub_topic, line.as_bytes()),
                Async::Ready(None) => panic!("Stdin closed"),
                Async::NotReady => break,
            };
        }

        loop {
            match swarm.poll().expect("Error while polling swarm") {
                Async::Ready(Some(message)) => {
                    println!("Received: {:?}", String::from_utf8_lossy(&message.data));
                },
                Async::Ready(None) | Async::NotReady => break,
            }
        }

        Ok(Async::NotReady)
    }));
}
