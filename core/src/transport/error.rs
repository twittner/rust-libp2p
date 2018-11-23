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

use crate::upgrade::UpgradeError;
use std::{fmt, io};

#[derive(Debug)]
pub enum Error<T, U> {
    /// Some I/O related error.
    Io(io::Error),
    /// Error procuced by a transport.
    Transport(T),
    /// Error during protocol upgrade
    Upgrade(UpgradeError<U>),

    #[doc(hidden)]
    __Nonexhaustive
}

impl<T, U> Error<T, U> {
    pub fn map_transport_err<F, E>(self, f: F) -> Error<E, U>
    where
        F: FnOnce(T) -> E
    {
        match self {
            Error::Io(e) => Error::Io(e),
            Error::Transport(e) => Error::Transport(f(e)),
            Error::Upgrade(e) => Error::Upgrade(e),
            Error::__Nonexhaustive => Error::__Nonexhaustive
        }
    }

    pub fn map_upgrade_err<F, E>(self, f: F) -> Error<T, E>
    where
        F: FnOnce(U) -> E
    {
        match self {
            Error::Io(e) => Error::Io(e),
            Error::Transport(e) => Error::Transport(e),
            Error::Upgrade(e) => Error::Upgrade(e.map_err(f)),
            Error::__Nonexhaustive => Error::__Nonexhaustive
        }
    }
}

impl<T, U> fmt::Display for Error<T, U>
where
    T: fmt::Display,
    U: fmt::Display
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "i/o error: {}", e),
            Error::Transport(e) => write!(f, "transport error: {}", e),
            Error::Upgrade(e) => write!(f, "upgrade error: {}", e),
            Error::__Nonexhaustive => f.write_str("__Nonexhaustive")
        }
    }
}

impl<T, U> std::error::Error for Error<T, U>
where
    T: std::error::Error,
    U: std::error::Error
{
    fn cause(&self) -> Option<&dyn std::error::Error> {
        match self {
            Error::Io(e) => Some(e),
            Error::Transport(e) => Some(e),
            Error::Upgrade(e) => Some(e),
            Error::__Nonexhaustive => None
        }
    }
}

impl<T, U> From<UpgradeError<U>> for Error<T, U> {
    fn from(e: UpgradeError<U>) -> Self {
        Error::Upgrade(e)
    }
}

impl<T, U> From<io::Error> for Error<T, U> {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

