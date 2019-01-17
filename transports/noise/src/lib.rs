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

mod error;

use crate::error::NoiseError;
use curve25519_dalek::{
    constants::ED25519_BASEPOINT_POINT,
    edwards::CompressedEdwardsY,
    scalar::Scalar
};
use futures::{future::FutureResult, prelude::*};
use libp2p_core::{Multiaddr, Transport, TransportError};
use snow;
use std::{io, sync::Arc};
use tokio_io::{AsyncRead, AsyncWrite};

pub type Result<T> = std::result::Result<T, NoiseError>;

const PATTERN: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2b";
const MAX_MSG_BUF: usize = 64000;

pub struct Keypair {
    secret: Scalar,
    public: CompressedEdwardsY
}

impl Keypair {
    pub fn fresh() -> Self {
        let s = Scalar::random(&mut rand::thread_rng());
        let p = s * ED25519_BASEPOINT_POINT;
        Keypair { secret: s, public: p.compress() }
    }

    pub fn secret(&self) -> &[u8; 32] {
        self.secret.as_bytes()
    }

    pub fn public(&self) -> &[u8; 32] {
        self.public.as_bytes()
    }
}

#[derive(Clone)]
pub struct NoiseConfig<T> {
    keypair: Arc<Keypair>,
    params: snow::params::NoiseParams,
    transport: T
}

impl<T: Transport> NoiseConfig<T> {
    pub fn new(transport: T, kp: Keypair) -> Self {
        NoiseConfig {
            keypair: Arc::new(kp),
            params: PATTERN.parse().expect("constant pattern always parses successfully"),
            transport
        }
    }
}

impl<T: Transport> Transport for NoiseConfig<T> {
    type Output = NoiseOutput<T::Output>;
    type Error = NoiseError;
    type Listener = NoiseListener<T::Listener>;
    type ListenerUpgrade = FutureResult<Self::Output, NoiseError>;
    type Dial = NoiseDial<T::Dial>;

    fn listen_on(self, addr: Multiaddr) -> std::result::Result<(Self::Listener, Multiaddr), TransportError<Self::Error>> {
        unimplemented!()
    }

    fn dial(self, addr: Multiaddr) -> std::result::Result<Self::Dial, TransportError<Self::Error>> {
        unimplemented!()
    }

    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        self.transport.nat_traversal(server, observed)
    }
}

pub struct NoiseListener<T> {
    listener: T
}

impl<T: Stream> Stream for NoiseListener<T> {
    type Item = (); //TODO!
    type Error= NoiseError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        unimplemented!()
    }
}

pub struct NoiseDial<T> {
    dial: T
}

impl<T: Future> Future for NoiseDial<T> {
    type Item = NoiseOutput<T::Item>;
    type Error = NoiseError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        unimplemented!()
    }
}

pub struct NoiseOutput<T> {
    io: T,
    session: snow::Session,
    buf_enc: Box<[u8; 65535]>,
    buf_dec: Box<[u8; 65535]>,
    state: State
}

enum State {
    /// initial state
    Init,
    /// read encrypted frame data
    ReadData { len: usize, off: usize },
    /// copy decrypted frame data
    CopyData { len: usize, off: usize },
    /// accumulate write data
    AccData { len: usize },
    /// write out encrypted data
    WriteData { len: usize, off: usize },
    /// end of file has been reached (terminal state)
    EOF,
    /// decryption error (terminal state)
    DecErr,
    /// encryption error (terminal state)
    EncErr,
    /// invalid state error (terminal state)
    InvState
}

impl<T: io::Read> io::Read for NoiseOutput<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        use byteorder::{BigEndian, ReadBytesExt};
        loop {
            match self.state {
                State::Init => {
                    let n = self.io.read_u16::<BigEndian>()?;
                    if n == 0 {
                        self.state = State::EOF;
                        return Ok(0)
                    }
                    self.state = State::ReadData { len: usize::from(n), off: 0 }
                }
                State::ReadData { len, ref mut off } => {
                    let n = self.io.read(&mut self.buf_enc[*off..])?;
                    if n == 0 {
                        self.state = State::EOF;
                        return Ok(0)
                    }
                    *off += n;
                    if len == *off {
                        if let Ok(n) = self.session.read_message(&self.buf_enc[..len], &mut self.buf_dec[..]) {
                            self.state = State::CopyData { len: n, off: 0 }
                        } else {
                            self.state = State::DecErr;
                            return Err(io::ErrorKind::InvalidData.into())
                        }
                    }
                }
                State::CopyData { len, ref mut off } => {
                    let n = std::cmp::min(len - *off, buf.len());
                    (&mut buf[..n]).copy_from_slice(&self.buf_dec[*off..len]);
                    *off += n;
                    if len == *off {
                        self.state = State::Init
                    }
                    return Ok(n)
                }
                State::AccData {..} | State::WriteData {..} | State::EncErr => {
                    self.state = State::InvState
                }
                State::EOF => return Ok(0),
                State::DecErr => return Err(io::ErrorKind::InvalidData.into()),
                State::InvState => return Err(io::Error::new(io::ErrorKind::Other, "invalid state"))
            }
        }
    }
}

impl<T: io::Write> io::Write for NoiseOutput<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        loop {
            match self.state {
                State::Init => {
                    self.state = State::AccData { len: 0 }
                }
                State::AccData { ref mut len } => {
                    let n = std::cmp::min(MAX_MSG_BUF - *len, buf.len());
                    (&mut self.buf_dec[*len .. *len + n]).copy_from_slice(&buf[..n]);
                    *len += n;
                    if *len == MAX_MSG_BUF {
                        if let Ok(n) = self.session.write_message(&self.buf_dec[..*len], &mut self.buf_enc[..]) {
                            self.state = State::WriteData { len: n, off: 0 }
                        } else {
                            self.state = State::EncErr;
                            return Err(io::ErrorKind::InvalidData.into())
                        }
                    }
                    return Ok(n)
                }
                State::WriteData { len, ref mut off } => {
                    let n = self.io.write(&self.buf_enc[*off..len])?;
                    if n == 0 {
                        self.state = State::EOF;
                        return Ok(0)
                    }
                    *off += n;
                    if len == *off {
                        self.state = State::Init
                    }
                }
                State::ReadData {..} | State::CopyData {..} | State::DecErr => {
                    self.state = State::InvState
                }
                State::EOF => return Ok(0),
                State::EncErr => return Err(io::ErrorKind::InvalidData.into()),
                State::InvState => return Err(io::Error::new(io::ErrorKind::Other, "invalid state"))
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        loop {
            match self.state {
                State::AccData { len } => {
                    if let Ok(n) = self.session.write_message(&self.buf_dec[..len], &mut self.buf_enc[..]) {
                        self.state = State::WriteData { len: n, off: 0 }
                    } else {
                        self.state = State::EncErr;
                        return Err(io::ErrorKind::InvalidData.into())
                    }
                }
                State::WriteData { len, ref mut off } => {
                    let n = self.io.write(&self.buf_enc[*off..len])?;
                    if n == 0 {
                        self.state = State::EOF;
                        return Ok(())
                    }
                    *off += n;
                    if len == *off {
                        self.state = State::Init;
                        return Ok(())
                    }
                }
                _ => return Ok(())
            }
        }
    }
}

impl<T: AsyncRead> AsyncRead for NoiseOutput<T> {}

impl<T: AsyncWrite> AsyncWrite for NoiseOutput<T> {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.io.shutdown()
    }
}
