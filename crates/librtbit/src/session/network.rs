use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, bail};
use futures::{StreamExt, TryFutureExt, stream::FuturesUnordered};
use librqbit_utp::BindDevice;
use tokio::sync::Semaphore;
use tracing::{Instrument, debug, debug_span, trace, warn};

/// Maximum number of incoming connection handshakes that can run concurrently.
/// This prevents FD exhaustion during connection bursts.
const MAX_CONCURRENT_HANDSHAKES: usize = 64;

use crate::{
    listen::Accept,
    read_buf::ReadBuf,
    stream_connect::ConnectionKind,
    torrent_state::TorrentStateLive,
    type_aliases::{BoxAsyncReadVectored, BoxAsyncWrite},
};

use super::Session;
use super::types::CheckedIncomingConnection;

impl Session {
    /// Ban peers by IP for the rest of this session's life. New connections
    /// to or from a banned IP are refused. Returns how many were newly added.
    pub fn ban_peers(&self, ips: impl IntoIterator<Item = IpAddr>) -> usize {
        let mut banned = self.banned_peers.write();
        let mut added = 0;
        for ip in ips {
            let ip = ip.to_canonical();
            if banned.insert(ip) {
                added += 1;
                crate::log_ring::global().push_peer(ip, true, "manually banned");
            }
        }
        added
    }

    pub fn is_peer_banned(&self, ip: IpAddr) -> bool {
        self.banned_peers.read().contains(&ip.to_canonical())
    }

    pub fn banned_peers(&self) -> Vec<IpAddr> {
        let mut ips: Vec<IpAddr> = self.banned_peers.read().iter().copied().collect();
        ips.sort();
        ips
    }

    pub(super) async fn check_incoming_connection(
        self: Arc<Self>,
        addr: SocketAddr,
        kind: ConnectionKind,
        mut reader: BoxAsyncReadVectored,
        writer: BoxAsyncWrite,
    ) -> anyhow::Result<(Arc<TorrentStateLive>, CheckedIncomingConnection)> {
        let rwtimeout = self
            .peer_opts
            .read_write_timeout
            .unwrap_or_else(|| Duration::from_secs(10));

        let incoming_ip = addr.ip();
        if self.blocklist.read().has(incoming_ip) {
            self.stats
                .counters
                .blocked_incoming
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            bail!("Incoming ip {incoming_ip} is in blocklist");
        }
        if self.allowlist.as_ref().is_some_and(|l| !l.has(incoming_ip)) {
            self.stats
                .counters
                .blocked_incoming
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            bail!("Incoming ip {incoming_ip} is not in allowlist");
        }
        if self.is_peer_banned(incoming_ip) {
            self.stats
                .counters
                .blocked_incoming
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            crate::log_ring::global().push_peer(incoming_ip, true, "banned peer (incoming)");
            bail!("Incoming ip {incoming_ip} is banned");
        }

        let mut read_buf = ReadBuf::new();
        let h = read_buf
            .read_handshake(&mut reader, rwtimeout)
            .await
            .context("error reading handshake")?;
        trace!("received handshake from {addr}: {:?}", h);

        if h.peer_id == self.peer_id {
            bail!("seems like we are connecting to ourselves, ignoring");
        }

        let (id, torrent) = self
            .db
            .read()
            .get_by_info_hash(h.info_hash)
            .map(|(id, t)| (id, t.clone()))
            .with_context(|| format!("didn't find a matching torrent {:?}", h.info_hash))?;

        let live = torrent
            .live_wait_initializing(Duration::from_secs(5))
            .await
            .with_context(|| format!("torrent {id} is not live, ignoring connection"))?;

        Ok((
            live,
            CheckedIncomingConnection {
                addr,
                reader,
                writer,
                kind,
                handshake: h,
                read_buf,
            },
        ))
    }

    pub(super) async fn task_listener<A: Accept>(self: Arc<Self>, l: A) -> anyhow::Result<()> {
        let mut futs = FuturesUnordered::new();
        let session = Arc::downgrade(&self);
        let handshake_sem = Arc::new(Semaphore::new(MAX_CONCURRENT_HANDSHAKES));
        drop(self);

        loop {
            tokio::select! {
                r = l.accept() => {
                    match r {
                        Ok((addr, (read, write))) => {
                            let permit = match handshake_sem.clone().try_acquire_owned() {
                                Ok(p) => p,
                                Err(_) => {
                                    debug!("max concurrent handshakes ({MAX_CONCURRENT_HANDSHAKES}) reached, dropping connection from {addr}");
                                    continue;
                                }
                            };
                            trace!("accepted connection from {addr}");
                            let session = session.upgrade().context("session is dead")?;
                            let span = debug_span!(parent: session.rs(), "incoming", addr=%addr);
                            futs.push(
                                async move {
                                    let result = session.check_incoming_connection(addr, A::KIND, Box::new(read), Box::new(write))
                                        .map_err(|e| {
                                            debug!("error checking incoming connection: {e:#}");
                                            e
                                        })
                                        .await;
                                    drop(permit);
                                    result
                                }
                                .instrument(span)
                            );
                        }
                        Err(e) => {
                            warn!("error accepting: {e:#}");
                            // Whatever is the reason, ensure we are not stuck trying to
                            // accept indefinitely.
                            tokio::time::sleep(Duration::from_secs(10)).await;
                            continue
                        }
                    }
                },
                Some(Ok((live, checked))) = futs.next(), if !futs.is_empty() => {
                    let (addr, kind) = (checked.addr, checked.kind);
                    if let Err(e) = live.add_incoming_peer(checked) {
                        warn!(?addr, ?kind, "error handing over incoming connection: {e:#}");
                    }
                },
            }
        }
    }

    pub(super) async fn task_upnp_port_forwarder(
        port: u16,
        bind_device: Option<BindDevice>,
    ) -> anyhow::Result<()> {
        let pf = librtbit_upnp::UpnpPortForwarder::new(vec![port], None, bind_device)?;
        pf.run_forever().await
    }
}
