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

use crate::{
    Multiaddr, PeerId, MultiaddrSeq,
    nodes::raw_swarm::ConnectedPoint,
    protocols_handler::ProtocolsHandler,
    swarm::{NetworkBehaviour, NetworkBehaviourAction}
};
use futures::prelude::*;
use void::Void;

pub struct EventReporter<Sub> {
    event: Option<Event<Void>>,
    _mark: std::marker::PhantomData<Sub>
}

impl<Sub> EventReporter<Sub> {
    pub fn new() -> Self {
        EventReporter {
            event: None,
            _mark: std::marker::PhantomData
        }
    }
}

#[derive(Clone, Debug)]
pub enum Event<T> {
    Listener {
        addr: Multiaddr,
        listens: MultiaddrSeq
    },
    Connected {
        peer: PeerId,
        endpoint: ConnectedPoint
    },
    Disconnected {
        peer: PeerId,
        endpoint: ConnectedPoint
    },
    NodeEvent {
        peer: PeerId,
        event: T
    }
}

impl<Top, Sub> NetworkBehaviour<Top> for EventReporter<Sub>
where
    Sub: tokio_io::AsyncRead + tokio_io::AsyncWrite
{
    type ProtocolsHandler = crate::protocols_handler::DummyProtocolsHandler<Sub>;
    type OutEvent = Event<Void>;

    fn new_handler(&mut self) -> Self::ProtocolsHandler {
        crate::protocols_handler::DummyProtocolsHandler::default()
    }

    fn inject_listener(&mut self, a: Multiaddr, b: MultiaddrSeq) {
        self.event = Some(Event::Listener { addr: a, listens: b })
    }

    fn inject_connected(&mut self, p: PeerId, c: ConnectedPoint) {
        self.event = Some(Event::Connected { peer: p, endpoint: c })
    }

    fn inject_disconnected(&mut self, p: &PeerId, c: ConnectedPoint) {
        self.event = Some(Event::Disconnected { peer: p.clone(), endpoint: c })
    }

    fn inject_node_event(&mut self, p: PeerId, e: <Self::ProtocolsHandler as ProtocolsHandler>::OutEvent) {
        self.event = Some(Event::NodeEvent { peer: p, event: e })
    }

    fn poll(&mut self, _: &mut Top)
        -> Async<NetworkBehaviourAction<<Self::ProtocolsHandler as ProtocolsHandler>::InEvent, Self::OutEvent>>
    {
        if let Some(e) = self.event.take() {
            Async::Ready(NetworkBehaviourAction::GenerateEvent(e))
        } else {
            Async::NotReady
        }
    }
}

