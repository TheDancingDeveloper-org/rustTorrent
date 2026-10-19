use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
    num::NonZeroU32,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use parking_lot::RwLock;
use peer_binary_protocol::extended;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc::Receiver};
use tracing::{Instrument, debug, debug_span, trace};

use crate::{
    Error,
    peer_connection::{PeerConnection, PeerConnectionOptions, WriterRequest},
    session::CheckedIncomingConnection,
};

use super::{
    TorrentStateLive,
    peer::{PeerRx, PeerTx, stats::atomic::PeerCountersAtomic as AtomicPeerCounters},
    peer_handler::{PeerHandler, PeerHandlerLocked},
};

impl TorrentStateLive {
    pub(crate) async fn task_upload_scheduler(
        self: Arc<Self>,
        mut rx: tokio::sync::mpsc::UnboundedReceiver<(
            tokio::sync::mpsc::UnboundedSender<WriterRequest>,
            librtbit_core::lengths::ChunkInfo,
        )>,
    ) -> crate::Result<()> {
        while let Some((tx, ci)) = rx.recv().await {
            tokio::select! {
                _ = tx.closed() => {
                    continue;
                }
                res = self.ratelimits.prepare_for_upload(NonZeroU32::new(ci.size).unwrap()) => {
                    res?;
                }
            };
            if let Some(session) = self.shared.session.upgrade() {
                tokio::select! {
                    _ = tx.closed() => {
                        continue;
                    }
                    res = session.ratelimits.prepare_for_upload(NonZeroU32::new(ci.size).unwrap()) => {
                        res?;
                    }
                }
            }
            let _ = tx.send(WriterRequest::ReadChunkRequest(ci));
        }
        Ok(())
    }

    pub(crate) async fn task_manage_incoming_peer(
        self: Arc<Self>,
        checked_peer: CheckedIncomingConnection,
        counters: Arc<AtomicPeerCounters>,
        tx: PeerTx,
        rx: PeerRx,
        permit: OwnedSemaphorePermit,
    ) -> crate::Result<()> {
        // Hold the permit as an RAII guard so it is always released,
        // even if we return early via `?` or panic.
        let _permit_guard = permit;

        let handler = PeerHandler {
            addr: checked_peer.addr,
            incoming: true,
            on_bitfield_notify: Default::default(),
            unchoke_notify: Default::default(),
            _locked: RwLock::new(PeerHandlerLocked { i_am_choked: true }),
            requests_sem: Semaphore::new(0),
            state: self.clone(),
            tx,
            counters,
            first_message_received: AtomicBool::new(false),
            cancel_token: self.cancellation_token.child_token(),
        };
        let _token_guard = handler.cancel_token.clone().drop_guard();
        let options = PeerConnectionOptions {
            connect_timeout: self.shared.options.peer_connect_timeout,
            read_write_timeout: self.shared.options.peer_read_write_timeout,
            ..Default::default()
        };
        let peer_connection = PeerConnection::new(
            checked_peer.addr,
            self.shared.info_hash,
            self.shared.peer_id,
            &handler,
            Some(options),
            self.shared.spawner.clone(),
            self.shared.connector.clone(),
        );
        let requester = handler.task_peer_chunk_requester();

        let res = tokio::select! {
            r = requester => {r}
            r = peer_connection.manage_peer_incoming(
                rx,
                checked_peer,
                self.have_broadcast_tx.subscribe()
            ) => {r}
        };

        match res {
            // We disconnected the peer ourselves as we don't need it
            Ok(()) => {
                handler.on_peer_died(None)?;
            }
            Err(e) => {
                debug!("error managing peer: {:#}", e);
                handler.on_peer_died(Some(e))?;
            }
        };
        Ok(())
    }

    pub(crate) async fn task_manage_outgoing_peer(
        self: Arc<Self>,
        addr: SocketAddr,
        permit: OwnedSemaphorePermit,
    ) -> crate::Result<()> {
        // Hold the permit as an RAII guard so it is always released,
        // even if we return early via `?` or panic.
        let _permit_guard = permit;

        let state = self;
        let (rx, tx) = state.peers.mark_peer_connecting(addr)?;
        let counters = state
            .peers
            .with_peer(addr, |p| p.stats.counters.clone())
            .ok_or(Error::BugPeerNotFound)?;

        let handler = PeerHandler {
            addr,
            incoming: false,
            on_bitfield_notify: Default::default(),
            unchoke_notify: Default::default(),
            _locked: RwLock::new(PeerHandlerLocked { i_am_choked: true }),
            requests_sem: Semaphore::new(0),
            state: state.clone(),
            tx,
            counters,
            first_message_received: AtomicBool::new(false),
            cancel_token: state.cancellation_token.child_token(),
        };
        let _token_guard = handler.cancel_token.clone().drop_guard();

        let options = PeerConnectionOptions {
            connect_timeout: state.shared.options.peer_connect_timeout,
            read_write_timeout: state.shared.options.peer_read_write_timeout,
            ..Default::default()
        };
        let peer_connection = PeerConnection::new(
            addr,
            state.shared.info_hash,
            state.shared.peer_id,
            &handler,
            Some(options),
            state.shared.spawner.clone(),
            state.shared.connector.clone(),
        );
        let requester = aframe!(
            handler
                .task_peer_chunk_requester()
                .instrument(debug_span!("chunk_requester"))
        );
        let conn_manager = aframe!(
            peer_connection
                .manage_peer_outgoing(rx, state.have_broadcast_tx.subscribe())
                .instrument(debug_span!("peer_connection"))
        );

        handler
            .counters
            .outgoing_connection_attempts
            .fetch_add(1, Ordering::Relaxed);
        let res = tokio::select! {
            r = requester => {r}
            r = conn_manager => {r}
        };

        match res {
            // We disconnected the peer ourselves as we don't need it
            Ok(()) => {
                handler.on_peer_died(None)?;
            }
            Err(e) => {
                debug!("error managing peer: {:#}", e);
                handler.on_peer_died(Some(e))?;
            }
        }
        Ok(())
    }

    pub(crate) async fn task_peer_adder(
        self: Arc<Self>,
        mut peer_queue_rx: Receiver<SocketAddr>,
    ) -> crate::Result<()> {
        let state = self;
        loop {
            let addr = peer_queue_rx.recv().await.ok_or(Error::TorrentIsNotLive)?;
            if state.shared.options.disable_upload() && state.is_finished_and_no_active_streams() {
                debug!(?addr, "ignoring peer as we are finished");
                state.peers.mark_peer_not_needed(addr);
                continue;
            }

            let session = state
                .shared
                .session
                .upgrade()
                .ok_or(Error::SessionDestroyed)?;

            if session.ipv4_only && addr.is_ipv6() {
                debug!(?addr, "skipping ipv6 peer (ipv4_only=true)");
                continue;
            }

            if addr.port() == 0 {
                debug!(?addr, "skipping peer with port 0");
                continue;
            }

            if session.blocklist.has(addr.ip()) {
                session
                    .stats
                    .counters
                    .blocked_outgoing
                    .fetch_add(1, Ordering::Relaxed);
                debug!(?addr, "blocked outgoing connection (by the blacklist)");
                continue;
            }

            if session
                .allowlist
                .as_ref()
                .is_some_and(|l| !l.has(addr.ip()))
            {
                session
                    .stats
                    .counters
                    .blocked_outgoing
                    .fetch_add(1, Ordering::Relaxed);
                debug!(?addr, "blocked outgoing connection (by the allowlist)");
                continue;
            }

            if session.is_peer_banned(addr.ip()) {
                session
                    .stats
                    .counters
                    .blocked_outgoing
                    .fetch_add(1, Ordering::Relaxed);
                crate::log_ring::global().push_peer(addr.ip(), true, "banned peer (outgoing)");
                debug!(?addr, "blocked outgoing connection (banned peer)");
                continue;
            }

            let permit = state.peer_semaphore.clone().acquire_owned().await?;
            state.spawn(
                debug_span!(parent: state.shared.span.clone(), "manage_peer", peer = ?addr),
                format!("[{}][addr={addr}]manage_peer", state.shared.id),
                aframe!(state.clone().task_manage_outgoing_peer(addr, permit)),
            );
        }
    }

    /// Periodically checks the number of active peers. When the count drops
    /// below [`super::MIN_PEERS_THRESHOLD`] and the torrent is not finished,
    /// it triggers re-discovery by notifying `rediscovery_notify`, which in
    /// turn causes the external peer adder to request new peers from DHT and
    /// trackers. It also re-queues dead peers so they can be retried.
    pub(crate) async fn task_peer_health_monitor(self: Arc<Self>) -> crate::Result<()> {
        use super::{MIN_PEERS_THRESHOLD, PEER_HEALTH_CHECK_INTERVAL, REDISCOVERY_COOLDOWN};
        use std::time::Instant;

        let mut last_rediscovery: Option<Instant> = None;

        loop {
            tokio::time::sleep(PEER_HEALTH_CHECK_INTERVAL).await;

            // Don't bother with re-discovery if the torrent is finished.
            if self.is_finished_and_no_active_streams() {
                trace!("peer health monitor: torrent finished, skipping check");
                continue;
            }

            let active_peers = self.get_active_peer_count();
            if active_peers >= MIN_PEERS_THRESHOLD {
                trace!(
                    active_peers,
                    threshold = MIN_PEERS_THRESHOLD,
                    "peer health monitor: sufficient peers"
                );
                continue;
            }

            // Enforce cooldown to prevent spamming.
            if let Some(last) = last_rediscovery
                && last.elapsed() < REDISCOVERY_COOLDOWN
            {
                trace!(
                    active_peers,
                    cooldown_remaining_ms = (REDISCOVERY_COOLDOWN - last.elapsed()).as_millis(),
                    "peer health monitor: rediscovery cooldown active"
                );
                continue;
            }

            debug!(
                id = self.shared.id,
                active_peers,
                threshold = MIN_PEERS_THRESHOLD,
                "peer health monitor: low peer count, triggering re-discovery"
            );

            last_rediscovery = Some(Instant::now());

            // Re-queue dead peers so they get another chance.
            self.requeue_dead_peers();

            // Signal to the external peer adder that we need new peers.
            self.rediscovery_notify.notify_waiters();
        }
    }

    pub(crate) async fn task_send_pex_to_peer(
        self: Arc<Self>,
        this_peer_addr: SocketAddr,
        tx: PeerTx,
    ) -> anyhow::Result<()> {
        // As per BEP 11 we should not send more than 50 peers at once
        // (here it also applies to fist message, should be OK as we anyhow really have more)
        const MAX_SENT_PEERS: usize = 50;
        // As per BEP 11 recommended interval is min 60 seconds
        const PEX_MESSAGE_INTERVAL: Duration = Duration::from_secs(60);

        let mut connected = Vec::with_capacity(MAX_SENT_PEERS);
        let mut dropped = Vec::with_capacity(MAX_SENT_PEERS);
        let mut peer_view_of_live_peers = HashSet::new();

        // Wait 10 seconds before sending the first message to assure that peer will stay with us
        tokio::time::sleep(Duration::from_secs(10)).await;

        let mut interval = tokio::time::interval(PEX_MESSAGE_INTERVAL);

        loop {
            interval.tick().await;

            // This task should die with the cancellation token, but check defensively just in case.
            if tx.is_closed() {
                return Ok(());
            }

            {
                let live_peers = self.peers.live_outgoing_peers.read();
                connected.clear();
                dropped.clear();

                connected.extend(
                    live_peers
                        .difference(&peer_view_of_live_peers)
                        .take(MAX_SENT_PEERS)
                        .copied(),
                );
                dropped.extend(
                    peer_view_of_live_peers
                        .difference(&live_peers)
                        .take(MAX_SENT_PEERS)
                        .copied(),
                );
            }

            trace!(connected_len = connected.len(), dropped_len = dropped.len());

            let peer_ip_non_local = match this_peer_addr.ip() {
                IpAddr::V4(a) => !a.is_loopback() && !a.is_private(),
                IpAddr::V6(a) => !a.is_loopback() && !a.is_unique_local(),
            };

            let other_ip_is_local = |addr: &IpAddr| match addr {
                IpAddr::V4(a) => a.is_loopback() || a.is_private(),
                IpAddr::V6(a) => {
                    a.is_loopback() || a.is_unicast_link_local() || a.is_unique_local()
                }
            };

            let filter = |addr: &SocketAddr| !(peer_ip_non_local && other_ip_is_local(&addr.ip()));

            // BEP 11 - Dont send closed if they are now in live
            // it's assured by mutual exclusion of two  above sets  if in sent_peers_live, it cannot be in addrs_live_to_sent,
            // and addrs_closed_to_sent are only filtered addresses from sent_peers_live

            if !connected.is_empty() || !dropped.is_empty() {
                let pex_msg = extended::ut_pex::UtPex::from_addrs(
                    connected.iter().copied(),
                    dropped.iter().copied(),
                );
                if tx.send(WriterRequest::UtPex(pex_msg)).is_err() {
                    return Ok(()); // Peer disconnected
                }

                for addr in &dropped {
                    peer_view_of_live_peers.remove(addr);
                }
                peer_view_of_live_peers.extend(connected.iter().filter(|a| filter(a)).copied());
            }
        }
    }
}
