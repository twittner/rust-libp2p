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

//use criterion::{Criterion, criterion_group, criterion_main};
use bencher::{benchmark_group, benchmark_main, Bencher};
use futures::{future, prelude::*, stream};
use libp2p_core::Transport;
use libp2p_secio::{SecioConfig, SecioKeyPair};
use libp2p_tcp::TcpConfig;
use std::{io, iter};
use tokio::codec::length_delimited::Builder;

fn run(bufsize: usize) {
    tokio::run(future::lazy(move || {
        let keypair = SecioKeyPair::ed25519_generated().unwrap();
        let secio = SecioConfig::new(keypair).buffer_size(bufsize);
        let transport = TcpConfig::new().with_upgrade(secio);

        let (listener, addr) = transport
            .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
            .unwrap();

        let server = listener
            .into_future()
            .map_err(|(e, _)| e)
            .and_then(|(c, _)| c.unwrap().0)
            .and_then(|output| {
                let io = Builder::new().new_framed(output.stream);
                io.concat2().map(|_| ())
            })
            .map_err(|e| panic!("server error: {}", e));

        tokio::spawn(server);

        let keypair = SecioKeyPair::ed25519_generated().unwrap();
        let secio = SecioConfig::new(keypair).buffer_size(bufsize);
        let transport = TcpConfig::new().with_upgrade(secio);

        transport.dial(addr).unwrap()
            .and_then(|output| {
                let io = Builder::new().new_framed(output.stream);
                let data = stream::iter_ok::<_, io::Error>(iter::repeat(b"xy"[..].into()).take(100));
                io.send_all(data).map(|_| ())
            })
            .map_err(|e| panic!("client error: {}", e))
    }))
}

fn a(b: &mut Bencher) {
    let _ = env_logger::try_init();
    b.iter(|| run(0))
}

benchmark_group!(benches, a);
benchmark_main!(benches);

//fn buffered_benchmark(c: &mut Criterion) {
//    c.bench_function_over_inputs("buffered secio", |b, &&size| {
//        b.iter(|| run(size))
//    }, &[0, 128, 1024, 4096, 8192]);
//}
//
//criterion_group!(benches, buffered_benchmark);
//criterion_main!(benches);

