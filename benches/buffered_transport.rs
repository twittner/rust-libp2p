// Copyright 2019 Parity Technologies (UK) Ltd.
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

extern crate criterion;
extern crate futures;
extern crate libp2p;
extern crate tokio;

use criterion::{Criterion, criterion_group, criterion_main};
use futures::{future, prelude::*, stream};
use libp2p::Transport;
use std::{io, iter, sync::Arc};
use tokio::codec::length_delimited::Builder;

fn run(bufsize: usize) {
    tokio::run(future::lazy(move || {
        let transport = libp2p::tcp::TcpConfig::new()
            .buffered(bufsize)
            .with_upgrade(libp2p::mplex::MplexConfig::default());

        let (listener, addr) = transport
            .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
            .unwrap();

        let server = listener
            .into_future()
            .map_err(|(e, _)| e)
            .and_then(|(c, _)| c.unwrap().0)
            .and_then(|conn| {
                libp2p::core::muxing::inbound_from_ref_and_wrap(Arc::new(conn))
            })
            .and_then(|stream| {
                let io = Builder::new().new_framed(stream.unwrap());
                io.concat2().map(|_| ())
            })
            .map_err(|e| panic!("server error: {:?}", e));

        tokio::spawn(server);

        let transport = libp2p::tcp::TcpConfig::new()
            .buffered(bufsize)
            .with_upgrade(libp2p::mplex::MplexConfig::default());

        transport.dial(addr).unwrap()
            .and_then(|conn| libp2p::core::muxing::outbound_from_ref_and_wrap(Arc::new(conn)))
            .and_then(|stream| {
                let io = Builder::new().new_framed(stream.unwrap());
                let data = stream::iter_ok::<_, io::Error>(iter::repeat(b"xy"[..].into()).take(100));
                io.send_all(data).map(|_| ())
            })
            .map_err(|e| panic!("client error: {:?}", e))
    }))
}

fn buffered_transport_benchmark(c: &mut Criterion) {
    c.bench_function_over_inputs("buffered transport", |b, &&size| {
        b.iter(|| run(size))
    }, &[0, 16, 32, 64, 128]);
}

criterion_group!(benches, buffered_transport_benchmark);
criterion_main!(benches);

