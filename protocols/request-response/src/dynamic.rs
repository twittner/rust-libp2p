// Copyright 2020 Parity Technologies (UK) Ltd.
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

use crate::handler::{RequestProtocol, RequestResponseHandler, RequestResponseHandlerEvent};
use crate::codec::header;
use crate::throttled::Event;
use futures::ready;
use libp2p_core::{ConnectedPoint, connection::ConnectionId, Multiaddr, PeerId};
use libp2p_swarm::{NetworkBehaviour, NetworkBehaviourAction, PollParameters};
use std::{collections::{HashMap, HashSet, VecDeque}, task::{Context, Poll}};
use std::num::NonZeroU16;
use super::{
    RequestId,
    RequestResponse,
    RequestResponseCodec,
    RequestResponseEvent,
    RequestResponseMessage,
    ResponseChannel
};

/// A wrapper around [`RequestResponse`] which adds request limits per peer.
///
/// In contrast to the `Throttled` wrapper, this does not require static
/// agreement of the limits on both endpoints. Instead the limits are
/// dynamically discovered.
pub struct Dynamic<C>
where
    C: RequestResponseCodec + Send,
    C::Protocol: Sync
{
    /// A random id used for logging.
    id: u32,
    /// The wrapped behaviour.
    behaviour: RequestResponse<header::Codec<C>>,
    /// Information per peer.
    peer_info: HashMap<PeerId, PeerInfo>,
    /// The default limits apply to all peers unless overriden.
    default_limit: Limit,
    /// Permanent limit overrides per peer.
    limit_overrides: HashMap<PeerId, Limit>,
    /// Pending events to report in `Dynamic::poll`.
    events: VecDeque<Event<C::Request, C::Response, header::Response<C::Response>>>
}

/// Max. number of inbound requests that can be received.
#[derive(Clone, Copy, Debug)]
struct Limit {
    max_recv: NonZeroU16
}

/// The state of a response.
///
/// We track the additional credit in case sending back the response times out.
/// Then we need to try again with the next response.
#[derive(Clone, Copy, Debug)]
enum ResponseState {
    /// The response has not been sent yet.
    Pending,
    /// The response with the given additional credit is on its way.
    Sending(u16)
}

/// Information about a peer.
#[derive(Clone, Debug)]
struct PeerInfo {
    /// Limits that apply.
    limit: Limit,
    /// Remaining number of outbound requests that can be sent.
    rem_send: u16,
    /// Remaining number of inbound requests that can be received.
    rem_recv: u16,
    /// Additional number of inbound requests that can be received.
    add_recv: u16,
    /// Current set of outbound requests.
    outbound: HashSet<RequestId>,
    /// Current set of inbound requests and their state.
    inbound: HashMap<RequestId, ResponseState>
}

impl PeerInfo {
    fn new(limit: Limit) -> Self {
        PeerInfo {
            limit,
            rem_send: 1,
            rem_recv: limit.max_recv.get(),
            add_recv: limit.max_recv.get() - 1,
            outbound: HashSet::new(),
            inbound: HashMap::new()
        }
    }
}

impl<C> Dynamic<C>
where
    C: RequestResponseCodec + Send + Clone,
    C::Protocol: Sync
{
    /// Wrap an existing `RequestResponse` behaviour and apply send/recv limits.
    pub fn new(behaviour: RequestResponse<header::Codec<C>>) -> Self {
        Dynamic {
            id: rand::random(),
            behaviour,
            peer_info: HashMap::new(),
            default_limit: Limit { max_recv: NonZeroU16::new(1).expect("1 > 0") },
            limit_overrides: HashMap::new(),
            events: VecDeque::new()
        }
    }

    /// Override the global default limit.
    ///
    /// Applies to new peers only. See [`Dynamic::set_limit`] to override
    /// limits for individual peers.
    pub fn set_default_limit(&mut self, limit: NonZeroU16) {
        log::trace!("{:08x}: new default limit: {:?}", self.id, limit);
        self.default_limit = Limit { max_recv: limit }
    }

    /// Increase the receive limit of a single peer.
    pub fn increase_receive_limit(&mut self, p: &PeerId, limit: NonZeroU16) {
        log::debug!("{:08x}: increase limit for {} to {:?}", self.id, p, limit);
        if let Some(info) = self.peer_info.get_mut(p) {
            if limit.get() <= info.limit.max_recv.get() {
                log::warn!("{:08x}: limit {} for {} does not increase", self.id, limit, p);
                return
            }
            let diff = limit.get() - info.limit.max_recv.get();
            // We grant more requests than so far. The `add_recv` value
            // will be added in the next response as extra credit so the
            // remote is aware of it.
            info.add_recv = diff;
            info.rem_recv += diff;
            info.limit.max_recv = limit
        }
        self.limit_overrides.insert(p.clone(), Limit { max_recv: limit } );
    }

    /// Remove any limit overrides for the given peer.
    pub fn remove_override(&mut self, p: &PeerId) {
        log::trace!("{:08x}: removing limit override for {}", self.id, p);
        self.limit_overrides.remove(p);
    }

    /// Has the limit of outbound requests been reached for the given peer?
    pub fn can_send(&mut self, p: &PeerId) -> bool {
        self.peer_info.get(p).map(|i| i.rem_send > 0).unwrap_or(true)
    }

    /// Send a request to a peer.
    ///
    /// If the limit of outbound requests has been reached, the request is
    /// returned. Sending more outbound requests should only be attempted
    /// once [`Event::ResumeSending`] has been received from [`NetworkBehaviour::poll`].
    pub fn send_request(&mut self, p: &PeerId, req: C::Request) -> Result<RequestId, C::Request> {
        log::trace!("{:08x}: sending request to {}", self.id, p);

        let info =
            if let Some(info) = self.peer_info.get_mut(p) {
                info
            } else {
                let limit = self.limit_overrides.get(p).copied().unwrap_or(self.default_limit);
                self.peer_info.entry(p.clone()).or_insert(PeerInfo::new(limit))
            };

        if info.rem_send == 0 {
            log::trace!("{:08x}: no budget to send request to {}", self.id, p);
            return Err(req)
        }

        info.rem_send -= 1;

        let rid = self.behaviour.send_request(p, header::Request::new(req));
        info.outbound.insert(rid);
        Ok(rid)
    }

    /// Answer an inbound request with a response.
    ///
    /// See [`RequestResponse::send_response`] for details.
    pub fn send_response(&mut self, ch: ResponseChannel<header::Response<C::Response>>, res: C::Response) {
        log::trace!("{:08x}: sending response to request {:?} to peer {}", self.id, ch.request_id(), &ch.peer);
        let mut res = header::Response::new(res);
        if let Some(info) = self.peer_info.get_mut(&ch.peer) {
            if let Some(state) = info.inbound.get_mut(&ch.request_id()) {
                debug_assert!(matches!(state, ResponseState::Pending));
                res.set_credit(1 + info.add_recv);
                *state = ResponseState::Sending(info.add_recv);
                info.add_recv = 0
            }
        } else {
            res.set_credit(1);
        }
        log::trace!("{:08x}: request {:?} response credit = {}", self.id, ch.request_id(), res.header().credit);
        self.behaviour.send_response(ch, res)
    }

    /// Add a known peer address.
    ///
    /// See [`RequestResponse::add_address`] for details.
    pub fn add_address(&mut self, p: &PeerId, a: Multiaddr) {
        self.behaviour.add_address(p, a)
    }

    /// Remove a previously added peer address.
    ///
    /// See [`RequestResponse::remove_address`] for details.
    pub fn remove_address(&mut self, p: &PeerId, a: &Multiaddr) {
        self.behaviour.remove_address(p, a)
    }

    /// Are we connected to the given peer?
    ///
    /// See [`RequestResponse::is_connected`] for details.
    pub fn is_connected(&self, p: &PeerId) -> bool {
        self.behaviour.is_connected(p)
    }

    /// Are we waiting for a response to the given request?
    ///
    /// See [`RequestResponse::is_pending_outbound`] for details.
    pub fn is_pending_outbound(&self, p: &RequestId) -> bool {
        self.behaviour.is_pending_outbound(p)
    }

    /// Are we still processing and inbound request?
    ///
    /// See [`RequestResponse::is_pending_inbound`] for details.
    pub fn is_pending_inbound(&self, p: &RequestId) -> bool {
        self.behaviour.is_pending_inbound(p)
    }
}

impl<C> NetworkBehaviour for Dynamic<C>
where
    C: RequestResponseCodec + Send + Clone + 'static,
    C::Protocol: Sync
{
    type ProtocolsHandler = RequestResponseHandler<header::Codec<C>>;
    type OutEvent = Event<C::Request, C::Response, header::Response<C::Response>>;

    fn new_handler(&mut self) -> Self::ProtocolsHandler {
        self.behaviour.new_handler()
    }

    fn addresses_of_peer(&mut self, p: &PeerId) -> Vec<Multiaddr> {
        self.behaviour.addresses_of_peer(p)
    }

    fn inject_connection_established(&mut self, p: &PeerId, id: &ConnectionId, end: &ConnectedPoint) {
        self.behaviour.inject_connection_established(p, id, end)
    }

    fn inject_connection_closed(&mut self, p: &PeerId, id: &ConnectionId, end: &ConnectedPoint) {
        self.behaviour.inject_connection_closed(p, id, end)
    }

    fn inject_connected(&mut self, p: &PeerId) {
        log::trace!("{:08x}: connected to {}", self.id, p);
        self.behaviour.inject_connected(p);
        // The limit may have been added by `Dynamic::send_request` already.
        if !self.peer_info.contains_key(p) {
            let limit = self.limit_overrides.get(p).copied().unwrap_or(self.default_limit);
            self.peer_info.insert(p.clone(), PeerInfo::new(limit));
        }
    }

    fn inject_disconnected(&mut self, p: &PeerId) {
        log::trace!("{:08x}: disconnected from {}", self.id, p);
        self.peer_info.remove(p);
        self.behaviour.inject_disconnected(p)
    }

    fn inject_dial_failure(&mut self, p: &PeerId) {
        self.behaviour.inject_dial_failure(p)
    }

    fn inject_event(&mut self, p: PeerId, i: ConnectionId, e: RequestResponseHandlerEvent<header::Codec<C>>) {
        if let RequestResponseHandlerEvent::ResponseSent(r) = &e {
            if let Some(info) = self.peer_info.get_mut(&p) {
                if let Some(ResponseState::Sending(_)) = info.inbound.remove(r) {
                    log::trace!("{:08x}: request {:?} response complete", self.id, r);
                    debug_assert!(info.rem_recv < info.limit.max_recv.get());
                    info.rem_recv += 1;
                }
            }
        }
        self.behaviour.inject_event(p, i, e)
    }

    fn poll(&mut self, cx: &mut Context<'_>, params: &mut impl PollParameters)
        -> Poll<NetworkBehaviourAction<RequestProtocol<header::Codec<C>>, Self::OutEvent>>
    {
        if let Some(ev) = self.events.pop_front() {
            return Poll::Ready(NetworkBehaviourAction::GenerateEvent(ev))
        } else if self.events.capacity() > super::EMPTY_QUEUE_SHRINK_THRESHOLD {
            self.events.shrink_to_fit()
        }

        loop {
            let event = ready!(self.behaviour.poll(cx, params));

            match event {
                NetworkBehaviourAction::GenerateEvent(RequestResponseEvent::Message { ref peer, ref message }) =>
                    match message {
                        RequestResponseMessage::Response { ref request_id, ref response } =>
                            if let Some(info) = self.peer_info.get_mut(peer) {
                                if info.outbound.remove(request_id) {
                                    log::trace! {
                                        "{:08x}: received {} as credit from peer {}",
                                        self.id,
                                        response.header().credit,
                                        peer
                                    };
                                    if info.rem_send == 0 && response.header().credit > 0 {
                                        log::trace!("{:08x}: sending to peer {} can resume", self.id, peer);
                                        self.events.push_back(Event::ResumeSending(peer.clone()))
                                    }
                                    info.rem_send += response.header().credit
                                }
                            }

                        RequestResponseMessage::Request { ref request_id, .. } =>
                            if let Some(info) = self.peer_info.get_mut(peer) {
                                log::trace! {
                                    "{:08x}: received request {:?}, remaining budget = {}",
                                    self.id,
                                    request_id,
                                    info.rem_recv
                                };
                                if info.rem_recv == 0 {
                                    log::error!("{:08x}: peer {} exceeds its budget", self.id, peer);
                                    self.events.push_back(Event::TooManyInboundRequests(peer.clone()));
                                    continue
                                }
                                info.inbound.insert(*request_id, ResponseState::Pending);
                                info.rem_recv -= 1
                            }
                    }

                NetworkBehaviourAction::GenerateEvent(RequestResponseEvent::OutboundFailure {
                    ref peer,
                    ref request_id,
                    ..
                }) => {
                    if let Some(info) = self.peer_info.get_mut(peer) {
                        if info.outbound.remove(request_id) {
                            if info.rem_send == 0 {
                                log::trace!("{:08x}: sending to peer {} can resume", self.id, peer);
                                self.events.push_back(Event::ResumeSending(peer.clone()))
                            }
                            info.rem_send += 1
                        }
                    }
                }

                NetworkBehaviourAction::GenerateEvent(RequestResponseEvent::InboundFailure {
                    ref peer,
                    ref request_id,
                    ..
                }) => {
                    if let Some(info) = self.peer_info.get_mut(peer) {
                        if let Some(ResponseState::Sending(c)) = info.inbound.remove(request_id) {
                            log::trace! {
                                "{:08x}: failed to send request {:?} response with credit = {}",
                                self.id,
                                request_id,
                                c
                            };
                            debug_assert!(info.rem_recv < info.limit.max_recv.get());
                            info.rem_recv += 1;
                            info.add_recv += c
                        }
                    }
                }

                _ => ()
            };

            return Poll::Ready(event.map_out(|e| match e {
                RequestResponseEvent::Message { peer, message } => {
                    let message = match message {
                        RequestResponseMessage::Request { request_id, request, channel } =>
                            RequestResponseMessage::Request {
                                request_id,
                                request: request.into_parts().1,
                                channel
                            },
                        RequestResponseMessage::Response { request_id, response } =>
                            RequestResponseMessage::Response {
                                request_id,
                                response: response.into_parts().1
                            }
                    };
                    Event::Event(RequestResponseEvent::Message { peer, message })
                }
                RequestResponseEvent::OutboundFailure { peer, request_id, error } =>
                    Event::Event(RequestResponseEvent::OutboundFailure { peer, request_id, error }),
                RequestResponseEvent::InboundFailure { peer, request_id, error } =>
                    Event::Event(RequestResponseEvent::InboundFailure { peer, request_id, error })
            }))
        }
    }
}
