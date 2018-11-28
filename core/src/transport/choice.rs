// Copyright 2017 Parity Technologies (UK) Ltd.
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

use either::{EitherListenStream, EitherOutput, EitherFuture, EitherListenOn};
use multiaddr::Multiaddr;
use transport::Transport;

/// Struct returned by `or_transport()`.
#[derive(Debug, Copy, Clone)]
pub struct OrTransport<A, B>(A, B);

impl<A, B> OrTransport<A, B> {
    pub fn new(a: A, b: B) -> OrTransport<A, B> {
        OrTransport(a, b)
    }
}

impl<A, B> Transport for OrTransport<A, B>
where
    A: Transport,
    B: Transport,
{
    type Output = EitherOutput<A::Output, B::Output>;
    type ListenOn = EitherListenOn<A::ListenOn, B::ListenOn>;
    type Listener = EitherListenStream<A::Listener, B::Listener>;
    type ListenerUpgrade = EitherFuture<A::ListenerUpgrade, B::ListenerUpgrade>;
    type Dial = EitherFuture<A::Dial, B::Dial>;

    fn listen_on(self, addr: Multiaddr) -> Result<Self::ListenOn, (Self, Multiaddr)> {
        match self.0.listen_on(addr) {
            Ok(listen_on) => {
                Ok(EitherListenOn::A(listen_on))
            }
            Err((listener0, addr)) => {
                match self.1.listen_on(addr) {
                    Ok(listen_on) => Ok(EitherListenOn::B(listen_on)),
                    Err((listener1, addr)) => Err((OrTransport(listener0, listener1), addr))
                }
            }
        }
    }

    fn dial(self, addr: Multiaddr) -> Result<Self::Dial, (Self, Multiaddr)> {
        match self.0.dial(addr) {
            Ok(future) => {
                Ok(EitherFuture::First(future))
            }
            Err((dialer0, addr)) => {
                match self.1.dial(addr) {
                    Ok(future) => Ok(EitherFuture::Second(future)),
                    Err((dialer1, addr)) => Err((OrTransport(dialer0, dialer1), addr))
                }
            }
        }
    }

    #[inline]
    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        let first = self.0.nat_traversal(server, observed);
        if let Some(first) = first {
            return Some(first);
        }

        self.1.nat_traversal(server, observed)
    }
}
