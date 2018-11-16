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
use std::fmt;

#[derive(Debug)]
pub enum TransportError<T, U> {
    Transport(T),
    Upgrade(UpgradeError<U>),
    #[doc(hidden)]
    __Nonexhaustive
}

impl<T, U> TransportError<T, U> {
    pub fn map_transport_err<F, E>(self, f: F) -> TransportError<E, U>
    where
        F: FnOnce(T) -> E
    {
        match self {
            TransportError::Transport(t) => TransportError::Transport(f(t)),
            TransportError::Upgrade(e) => TransportError::Upgrade(e),
            TransportError::__Nonexhaustive => TransportError::__Nonexhaustive
        }
    }

    pub fn map_upgrade_err<F, E>(self, f: F) -> TransportError<T, E>
    where
        F: FnOnce(U) -> E
    {
        match self {
            TransportError::Transport(t) => TransportError::Transport(t),
            TransportError::Upgrade(e) => TransportError::Upgrade(e.map_err(f)),
            TransportError::__Nonexhaustive => TransportError::__Nonexhaustive
        }
    }
}

impl<T, U> fmt::Display for TransportError<T, U>
where
    T: fmt::Display,
    U: fmt::Display
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            TransportError::Transport(e) => write!(f, "transport error: {}", e),
            TransportError::Upgrade(e) => write!(f, "upgrade error: {}", e),
            TransportError::__Nonexhaustive => f.write_str("__Nonexhaustive")
        }
    }
}

impl<T, U> std::error::Error for TransportError<T, U>
where
    T: std::error::Error,
    U: std::error::Error
{
    fn cause(&self) -> Option<&dyn std::error::Error> {
        match self {
            TransportError::Transport(e) => Some(e),
            TransportError::Upgrade(e) => Some(e),
            TransportError::__Nonexhaustive => None
        }
    }
}
