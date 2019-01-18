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

use error::NoiseError;
use curve25519_dalek::{
    constants::X25519_BASEPOINT,
    montgomery::MontgomeryPoint,
    scalar::Scalar
};
use futures::{prelude::*, try_ready};
use libp2p_core::{multiaddr::{Multiaddr, Protocol}, PeerId, Transport, TransportError};
use log::{debug, trace};
use snow;
use std::{fmt,io, mem, sync::Arc};
use tokio_io::{AsyncRead, AsyncWrite};

const PATTERN: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";

#[derive(Clone, Debug)]
pub struct PublicKey(MontgomeryPoint);

impl PublicKey {
    pub fn base58_encoded(&self) -> String {
        bs58::encode(self.0.as_bytes()).into_string()
    }
}

impl AsRef<[u8]> for PublicKey {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

/// Curve25519 keypair.
pub struct Keypair {
    secret: Scalar,
    public: PublicKey
}

impl Keypair {
    pub fn fresh() -> Self {
        let s = Scalar::random(&mut rand::thread_rng());
        let p = s * X25519_BASEPOINT;
        Keypair { secret: s, public: PublicKey(p) }
    }

    pub fn secret(&self) -> &[u8; 32] {
        self.secret.as_bytes()
    }

    pub fn public(&self) -> &PublicKey {
        &self.public
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

impl<T> Transport for NoiseConfig<T>
where
    T: Transport,
    T::Output: AsyncRead + AsyncWrite,
    T::Error: 'static
{
    type Output = (PeerId, NoiseOutput<T::Output>);
    type Error = NoiseError<T::Error>;
    type Listener = NoiseListener<T::Listener>;
    type ListenerUpgrade = NoiseListenFuture<T::ListenerUpgrade>;
    type Dial = NoiseDialFuture<T::Dial>;

    fn listen_on(self, addr: Multiaddr) -> Result<(Self::Listener, Multiaddr), TransportError<Self::Error>> {
        let (listener, addr) = self.transport.listen_on(addr)
            .map_err(|e| match e {
                TransportError::MultiaddrNotSupported(a) => TransportError::MultiaddrNotSupported(a),
                TransportError::Other(e) => TransportError::Other(NoiseError::Inner(e))
            })?;

        debug!("listening on: {}", addr);

        Ok((NoiseListener { listener, keypair: self.keypair, params: self.params }, addr))
    }

    fn dial(self, mut addr: Multiaddr) -> Result<Self::Dial, TransportError<Self::Error>> {
        let pubkey =
            match addr.pop() {
                Some(Protocol::Curve25519(key)) => MontgomeryPoint(key.into_owned()),
                Some(proto) => {
                    addr.append(proto);
                    return Err(TransportError::MultiaddrNotSupported(addr))
                }
                None => return Err(TransportError::MultiaddrNotSupported(addr))
            };

        debug!("dialing {}", addr);

        let dial = self.transport.dial(addr)
            .map_err(|e| match e {
                TransportError::MultiaddrNotSupported(a) => TransportError::MultiaddrNotSupported(a),
                TransportError::Other(e) => TransportError::Other(NoiseError::Inner(e))
            })?;

        debug!("creating session with {}", PublicKey(pubkey.clone()).base58_encoded());

        let session = snow::Builder::new(self.params.clone())
            .local_private_key(self.keypair.secret())
            .remote_public_key(pubkey.as_bytes())
            .build_initiator()
            .map_err(|e| TransportError::Other(NoiseError::Noise(e)))?;

        Ok(NoiseDialFuture(DialState::Init(dial, session)))
    }

    fn nat_traversal(&self, server: &Multiaddr, observed: &Multiaddr) -> Option<Multiaddr> {
        self.transport.nat_traversal(server, observed)
    }
}

pub struct NoiseListener<T> {
    listener: T,
    keypair: Arc<Keypair>,
    params: snow::params::NoiseParams
}

impl<T, F> Stream for NoiseListener<T>
where
    T: Stream<Item = (F, Multiaddr)>,
    F: Future,
    F::Item: AsyncRead + AsyncWrite
{
    type Item = (NoiseListenFuture<F>, Multiaddr);
    type Error= NoiseError<T::Error>;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if let Some((future, addr)) = try_ready!(self.listener.poll().map_err(NoiseError::Inner)) {
            trace!("incoming stream: creating new session");
            let session = snow::Builder::new(self.params.clone())
                .local_private_key(self.keypair.secret())
                .build_responder()?;
            Ok(Async::Ready(Some((NoiseListenFuture(ListenState::Init(future, session)), addr))))
        } else {
            Ok(Async::Ready(None))
        }
    }
}

pub struct NoiseListenFuture<T: Future>(ListenState<T>);

enum ListenState<T: Future> {
    Init(T, snow::Session),
    RecvHandshake(NoiseOutput<T::Item>),
    SendHandshake(NoiseOutput<T::Item>),
    Flush(NoiseOutput<T::Item>),
    Done
}

impl<T> Future for NoiseListenFuture<T>
where
    T: Future,
    T::Item: AsyncRead + AsyncWrite
{
    type Item = (PeerId, NoiseOutput<T::Item>);
    type Error = NoiseError<T::Error>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            match mem::replace(&mut self.0, ListenState::Done) {
                ListenState::Init(mut future, session) => {
                    if let Async::Ready(io) = future.poll().map_err(NoiseError::Inner)? {
                        let output = NoiseOutput::new(io, session);
                        self.0 = ListenState::RecvHandshake(output)
                    } else {
                        mem::replace(&mut self.0, ListenState::Init(future, session));
                        return Ok(Async::NotReady)
                    }
                }
                ListenState::RecvHandshake(mut io) => {
                    // -> e, es, s, ss
                    if io.poll_read(&mut []).map_err(NoiseError::Io)?.is_ready() {
                        self.0 = ListenState::SendHandshake(io)
                    } else {
                        mem::replace(&mut self.0, ListenState::RecvHandshake(io));
                        return Ok(Async::NotReady)
                    }
                }
                ListenState::SendHandshake(mut io) => {
                    // <- e, ee, se
                    if io.poll_write(&[]).map_err(NoiseError::Io)?.is_ready() {
                        self.0 = ListenState::Flush(io)
                    } else {
                        mem::replace(&mut self.0, ListenState::SendHandshake(io));
                        return Ok(Async::NotReady)
                    }
                }
                ListenState::Flush(mut io) => {
                    if io.poll_flush().map_err(NoiseError::Io)?.is_ready() {
                        let s = io.session.into_transport_mode()?;
                        let m = s.get_remote_static()
                            .ok_or(NoiseError::InvalidKey)
                            .and_then(montgomery)?;
                        let io = NoiseOutput { session: s, .. io };
                        self.0 = ListenState::Done;
                        return Ok(Async::Ready((PeerId::encode(m.as_bytes()), io)))
                    } else {
                        mem::replace(&mut self.0, ListenState::Flush(io));
                        return Ok(Async::NotReady)
                    }
                }
                ListenState::Done => panic!("NoiseListenFuture::poll called after completion")
            }
        }
    }
}

pub struct NoiseDialFuture<T: Future>(DialState<T>);

enum DialState<T: Future> {
    Init(T, snow::Session),
    SendHandshake(NoiseOutput<T::Item>),
    Flush(NoiseOutput<T::Item>),
    RecvHandshake(NoiseOutput<T::Item>),
    Done
}

impl<T> Future for NoiseDialFuture<T>
where
    T: Future,
    T::Item: AsyncRead + AsyncWrite
{
    type Item = (PeerId, NoiseOutput<T::Item>);
    type Error = NoiseError<T::Error>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            match mem::replace(&mut self.0, DialState::Done) {
                DialState::Init(mut future, session) => {
                    if let Async::Ready(io) = future.poll().map_err(NoiseError::Inner)? {
                        let output = NoiseOutput::new(io, session);
                        self.0 = DialState::SendHandshake(output)
                    } else {
                        mem::replace(&mut self.0, DialState::Init(future, session));
                        return Ok(Async::NotReady)
                    }
                }
                DialState::SendHandshake(mut io) => {
                    // -> e, es, s, ss
                    if io.poll_write(&[]).map_err(NoiseError::Io)?.is_ready() {
                        self.0 = DialState::Flush(io)
                    } else {
                        mem::replace(&mut self.0, DialState::SendHandshake(io));
                        return Ok(Async::NotReady)
                    }
                }
                DialState::Flush(mut io) => {
                    if io.poll_flush().map_err(NoiseError::Io)?.is_ready() {
                        self.0 = DialState::RecvHandshake(io)
                    } else {
                        mem::replace(&mut self.0, DialState::Flush(io));
                        return Ok(Async::NotReady)
                    }
                }
                DialState::RecvHandshake(mut io) => {
                    // <- e, ee, se
                    if io.poll_read(&mut []).map_err(NoiseError::Io)?.is_ready() {
                        let s = io.session.into_transport_mode()?;
                        let m = s.get_remote_static()
                            .ok_or(NoiseError::InvalidKey)
                            .and_then(montgomery)?;
                        let io = NoiseOutput { session: s, .. io };
                        self.0 = DialState::Done;
                        return Ok(Async::Ready((PeerId::encode(m.as_bytes()), io)))
                    } else {
                        mem::replace(&mut self.0, DialState::RecvHandshake(io));
                        return Ok(Async::NotReady)
                    }
                }
                DialState::Done => panic!("NoiseDialFuture::poll called after completion")
            }
        }
    }
}

fn montgomery<E>(bytes: &[u8]) -> Result<MontgomeryPoint, NoiseError<E>> {
    if bytes.len() != 32 {
        return Err(NoiseError::InvalidKey)
    }
    let mut m = MontgomeryPoint([0; 32]);
    (&mut m.0).copy_from_slice(bytes);
    Ok(m)
}

const MAX_WRITE_BUF_LEN: usize = 16384;

pub struct NoiseOutput<T> {
    io: T,
    session: snow::Session,
    read_buf: Box<[u8; 65535]>, // incoming (encrypted) frames
    write_buf: Box<[u8; MAX_WRITE_BUF_LEN]>, // buffering data before encrypting & flushing
    read_crypto_buf: Box<[u8; 65535]>, // decrypted `read_buf` data goes in here
    write_crypto_buf: Box<[u8; 2 * MAX_WRITE_BUF_LEN]>, // encrypted `write_buf` data goes in here
    read_state: ReadState,
    write_state: WriteState
}

impl<T> fmt::Debug for NoiseOutput<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("NoiseOutput")
            .field("read_state", &self.read_state)
            .field("write_state", &self.write_state)
            .finish()
    }
}

impl<T> NoiseOutput<T> {
    fn new(io: T, session: snow::Session) -> Self {
        NoiseOutput {
            io, session,
            read_buf: Box::new([0; 65535]),
            write_buf: Box::new([0; MAX_WRITE_BUF_LEN]),
            read_crypto_buf: Box::new([0; 65535]),
            write_crypto_buf: Box::new([0; 2 * MAX_WRITE_BUF_LEN]),
            read_state: ReadState::Init,
            write_state: WriteState::Init
        }
    }
}

#[derive(Debug)]
enum ReadState {
    /// initial state
    Init,
    /// read encrypted frame data
    ReadData { len: usize, off: usize },
    /// copy decrypted frame data
    CopyData { len: usize, off: usize },
    /// end of file has been reached (terminal state)
    Eof,
    /// decryption error (terminal state)
    DecErr
}

#[derive(Debug)]
enum WriteState {
    /// initial state
    Init,
    /// accumulate write data
    BufferData { off: usize },
    /// write frame length
    WriteLen { len: usize },
    /// write out encrypted data
    WriteData { len: usize, off: usize },
    /// end of file has been reached (terminal state)
    Eof,
    /// encryption error (terminal state)
    EncErr
}

impl<T: io::Read> io::Read for NoiseOutput<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        use byteorder::{BigEndian, ReadBytesExt};
        loop {
            match self.read_state {
                ReadState::Init => {
                    let n = self.io.read_u16::<BigEndian>()?;
                    trace!("read: next frame len = {}", n);
                    if n == 0 {
                        self.read_state = ReadState::Eof;
                        return Ok(0)
                    }
                    self.read_state = ReadState::ReadData { len: usize::from(n), off: 0 }
                }
                ReadState::ReadData { len, ref mut off } => {
                    let n = self.io.read(&mut self.read_buf[*off .. len])?;
                    trace!("read: read {}/{} bytes", *off + n, len);
                    if n == 0 {
                        trace!("read: eof");
                        self.read_state = ReadState::Eof;
                        return Ok(0)
                    }
                    *off += n;
                    if len == *off {
                        trace!("read: decrypting {} bytes", len);
                        if let Ok(n) = self.session.read_message(&self.read_buf[.. len], &mut self.read_crypto_buf[..]) {
                            trace!("read: payload len = {} bytes", n);
                            self.read_state = ReadState::CopyData { len: n, off: 0 }
                        } else {
                            debug!("decryption error");
                            self.read_state = ReadState::DecErr;
                            return Err(io::ErrorKind::InvalidData.into())
                        }
                    }
                }
                ReadState::CopyData { len, ref mut off } => {
                    let n = std::cmp::min(len - *off, buf.len());
                    (&mut buf[.. n]).copy_from_slice(&self.read_crypto_buf[*off .. *off + n]);
                    trace!("read: copied {}/{} bytes", *off + n, len);
                    *off += n;
                    if len == *off {
                        self.read_state = ReadState::Init
                    }
                    return Ok(n)
                }
                ReadState::Eof => {
                    trace!("read: eof");
                    return Ok(0)
                }
                ReadState::DecErr => return Err(io::ErrorKind::InvalidData.into())
            }
        }
    }
}

impl<T: io::Write> io::Write for NoiseOutput<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        use byteorder::{BigEndian, WriteBytesExt};
        loop {
            match self.write_state {
                WriteState::Init => {
                    self.write_state = WriteState::BufferData { off: 0 }
                }
                WriteState::BufferData { ref mut off } => {
                    let n = std::cmp::min(MAX_WRITE_BUF_LEN - *off, buf.len());
                    (&mut self.write_buf[*off .. *off + n]).copy_from_slice(&buf[.. n]);
                    trace!("write: buffered {} bytes", *off + n);
                    *off += n;
                    if *off == MAX_WRITE_BUF_LEN {
                        trace!("write: encrypting {} bytes", *off);
                        if let Ok(n) = self.session.write_message(&self.write_buf[.. *off], &mut self.write_crypto_buf[..]) {
                            trace!("write: cipher text len = {} bytes", n);
                            self.write_state = WriteState::WriteLen { len: n }
                        } else {
                            debug!("encryption error");
                            self.write_state = WriteState::EncErr;
                            return Err(io::ErrorKind::InvalidData.into())
                        }
                    }
                    return Ok(n)
                }
                WriteState::WriteLen { len } => {
                    trace!("write: writing len ({})", len);
                    self.io.write_u16::<BigEndian>(len as u16)?;
                    self.write_state = WriteState::WriteData { len, off: 0 }
                }
                WriteState::WriteData { len, ref mut off } => {
                    let n = self.io.write(&self.write_crypto_buf[*off .. len])?;
                    trace!("write: wrote {}/{} bytes", *off + n, len);
                    if n == 0 {
                        trace!("write: eof");
                        self.write_state = WriteState::Eof;
                        return Ok(0)
                    }
                    *off += n;
                    if len == *off {
                        trace!("write: finished writing {} bytes", len);
                        self.write_state = WriteState::Init
                    }
                }
                WriteState::Eof => {
                    trace!("write: eof");
                    return Ok(0)
                }
                WriteState::EncErr => return Err(io::ErrorKind::InvalidData.into())
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        use byteorder::{BigEndian, WriteBytesExt};
        loop {
            match self.write_state {
                WriteState::Init => return Ok(()),
                WriteState::BufferData { off } => {
                    trace!("flush: encrypting {} bytes", off);
                    if let Ok(n) = self.session.write_message(&self.write_buf[.. off], &mut self.write_crypto_buf[..]) {
                        trace!("flush: cipher text len = {} bytes", n);
                        self.write_state = WriteState::WriteLen { len: n }
                    } else {
                        debug!("encryption error");
                        self.write_state = WriteState::EncErr;
                        return Err(io::ErrorKind::InvalidData.into())
                    }
                }
                WriteState::WriteLen { len } => {
                    trace!("flush: writing len ({})", len);
                    self.io.write_u16::<BigEndian>(len as u16)?;
                    self.write_state = WriteState::WriteData { len, off: 0 }
                }
                WriteState::WriteData { len, ref mut off } => {
                    let n = self.io.write(&self.write_crypto_buf[*off .. len])?;
                    trace!("flush: wrote {}/{} bytes", *off + n, len);
                    if n == 0 {
                        trace!("flush: eof");
                        self.write_state = WriteState::Eof;
                        return Ok(())
                    }
                    *off += n;
                    if len == *off {
                        trace!("flush: finished writing {} bytes", len);
                        self.write_state = WriteState::Init;
                        return Ok(())
                    }
                }
                WriteState::Eof => {
                    trace!("flush: eof");
                    return Ok(())
                }
                WriteState::EncErr => return Err(io::ErrorKind::InvalidData.into())
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
