use std::collections::HashSet;
use std::net::SocketAddr;
use std::net::SocketAddrV4;
use std::net::SocketAddrV6;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use anyhow::bail;
use backon::ExponentialBuilder;
use backon::Retryable;
use futures::FutureExt;
use futures::StreamExt;
use futures::future::Either;
use futures::stream::BoxStream;
use futures::stream::FuturesUnordered;
use tokio::sync::watch;
use tracing::Instrument;
use tracing::debug;
use tracing::debug_span;
use tracing::trace;
use tracing::trace_span;
use url::Url;

use crate::tracker_comms_http;
use crate::tracker_comms_udp;
use crate::tracker_comms_udp::UdpTrackerClient;
use crate::tracker_status::TrackerStatusRegistry;
use librtbit_core::AnnouncePort;
use librtbit_core::hash_id::Id20;

pub struct TrackerComms {
    info_hash: Id20,
    peer_id: Id20,
    stats: Box<dyn TorrentStatsProvider>,
    force_tracker_interval: Option<Duration>,
    tx: Sender,
    // This MUST be set as trackers don't work with 0 port.
    announce_port: AnnouncePort,
    reqwest_client: reqwest::Client,
    key: u32,
    status: Option<Arc<TrackerStatusRegistry>>,
}

#[derive(Default)]
pub enum TrackerCommsStatsState {
    #[default]
    None,
    Initializing,
    Paused,
    Live,
}

#[derive(Default)]
pub struct TrackerCommsStats {
    pub uploaded_bytes: u64,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub torrent_state: TrackerCommsStatsState,
}

impl TrackerCommsStats {
    pub fn get_left_to_download_bytes(&self) -> u64 {
        let total = self.total_bytes;
        let down = self.downloaded_bytes;
        if total >= down {
            return total - down;
        }
        0
    }

    pub fn is_completed(&self) -> bool {
        self.downloaded_bytes >= self.total_bytes
    }
}

pub trait TorrentStatsProvider: Send + Sync {
    fn get(&self) -> TrackerCommsStats;
}

impl TorrentStatsProvider for () {
    fn get(&self) -> TrackerCommsStats {
        Default::default()
    }
}

type Sender = tokio::sync::mpsc::Sender<SocketAddr>;

enum SupportedTracker {
    Udp(Url),
    Http(Url),
}

impl SupportedTracker {
    fn url(&self) -> &Url {
        match self {
            SupportedTracker::Udp(u) | SupportedTracker::Http(u) => u,
        }
    }
}

impl std::fmt::Debug for SupportedTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SupportedTracker::Udp(u) => std::fmt::Display::fmt(u, f),
            SupportedTracker::Http(u) => std::fmt::Display::fmt(u, f),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum UdpTrackerResolveResult {
    One(SocketAddr),
    Two(SocketAddrV4, SocketAddrV6),
}

async fn udp_tracker_to_socket_addrs(
    host: url::Host<&str>,
    port: u16,
) -> anyhow::Result<UdpTrackerResolveResult> {
    let res = match host {
        url::Host::Domain(name) => {
            // Use the first IPv4 and the first IPv6 addresses only.

            let mut v4: Option<SocketAddrV4> = None;
            let mut v6: Option<SocketAddrV6> = None;
            for addr in tokio::net::lookup_host((name, port))
                .await
                .with_context(|| format!("error looking up hostname {name}"))?
            {
                match (v4, v6, addr) {
                    (None, _, SocketAddr::V4(addr)) => v4 = Some(addr),
                    (_, None, SocketAddr::V6(addr)) => v6 = Some(addr),
                    _ => continue,
                }
            }
            let res = match (v4, v6) {
                (Some(v4), Some(v6)) => UdpTrackerResolveResult::Two(v4, v6),
                (Some(v4), None) => UdpTrackerResolveResult::One(v4.into()),
                (None, Some(v6)) => UdpTrackerResolveResult::One(v6.into()),
                _ => anyhow::bail!("zero addresses returned looking up {name}"),
            };
            trace!(?res, "resolved");
            res
        }
        url::Host::Ipv4(addr) => UdpTrackerResolveResult::One((addr, port).into()),
        url::Host::Ipv6(addr) => UdpTrackerResolveResult::One((addr, port).into()),
    };
    Ok(res)
}

impl TrackerComms {
    // TODO: fix too many args
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        info_hash: Id20,
        peer_id: Id20,
        trackers: HashSet<Url>,
        stats: Box<dyn TorrentStatsProvider>,
        force_interval: Option<Duration>,
        announce_port: u16,
        reqwest_client: reqwest::Client,
        udp_client: Option<UdpTrackerClient>,
        status: Option<Arc<TrackerStatusRegistry>>,
    ) -> Option<BoxStream<'static, SocketAddr>> {
        Self::start_with_shared_announce_port(
            info_hash,
            peer_id,
            trackers,
            stats,
            force_interval,
            AnnouncePort::new(std::num::NonZeroU16::new(announce_port)),
            reqwest_client,
            udp_client,
            status,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start_with_shared_announce_port(
        info_hash: Id20,
        peer_id: Id20,
        trackers: HashSet<Url>,
        stats: Box<dyn TorrentStatsProvider>,
        force_interval: Option<Duration>,
        announce_port: AnnouncePort,
        reqwest_client: reqwest::Client,
        udp_client: Option<UdpTrackerClient>,
        status: Option<Arc<TrackerStatusRegistry>>,
    ) -> Option<BoxStream<'static, SocketAddr>> {
        let trackers = trackers
            .into_iter()
            .filter_map(|t| match t.scheme() {
                "http" | "https" => Some(SupportedTracker::Http(t)),
                "udp" => match &udp_client {
                    Some(_) => Some(SupportedTracker::Udp(t)),
                    None => {
                        debug!("skipping UDP tracker {t}: proxy mode");
                        if let Some(s) = &status {
                            s.record_disabled(t.as_str(), "UDP trackers disabled (proxy mode)");
                        }
                        None
                    }
                },
                _ => {
                    debug!("unsupported tracker URL: {}", t);
                    if let Some(s) = &status {
                        s.record_disabled(t.as_str(), "unsupported tracker URL scheme");
                    }
                    None
                }
            })
            .collect::<Vec<_>>();
        if trackers.is_empty() {
            debug!(?info_hash, "trackers list is empty");
            return None;
        }

        // Capture each tracker's registry generation now, so a loop stops if
        // its tracker is removed (or removed and re-added) later.
        let trackers = trackers
            .into_iter()
            .map(|t| {
                let generation = status.as_ref().map(|s| s.ensure(t.url().as_str()));
                (t, generation)
            })
            .collect::<Vec<_>>();

        tracing::trace!(?trackers);

        let (tx, mut rx) = tokio::sync::mpsc::channel::<SocketAddr>(16);

        let s = async_stream::stream! {
            use futures::StreamExt;
            let comms = Arc::new(Self {
                info_hash,
                peer_id,
                stats,
                force_tracker_interval: force_interval,
                tx,
                announce_port,
                reqwest_client,
                key: rand::random(),
                status,
            });
            let mut futures = FuturesUnordered::new();
            for (tracker, generation) in trackers {
                futures.push(comms.add_tracker(tracker, generation, udp_client.as_ref()))
            }
            while !(futures.is_empty()) {
                tokio::select! {
                    addr = rx.recv() => {
                        if let Some(addr) = addr {
                            yield addr;
                        }
                    }
                    e = futures.next(), if !futures.is_empty() => {
                        if let Some(Err(e)) = e {
                            debug!("error: {e}");
                        }
                    }
                }
            }
        };

        Some(s.boxed())
    }

    fn add_tracker(
        &self,
        url: SupportedTracker,
        generation: Option<u64>,
        client: Option<&UdpTrackerClient>,
    ) -> Either<
        impl std::future::Future<Output = anyhow::Result<()>> + '_ + Send,
        impl std::future::Future<Output = anyhow::Result<()>> + '_ + Send,
    > {
        let info_hash = self.info_hash;
        match url {
            SupportedTracker::Udp(url) => {
                // Safe to unwrap: UDP trackers are filtered out when client is None
                let span = debug_span!(parent: None, "udp_tracker", tracker = %url, info_hash = ?info_hash);
                self.task_single_tracker_monitor_udp(url, generation, client.unwrap().clone())
                    .instrument(span)
                    .right_future()
            }
            SupportedTracker::Http(url) => {
                let span = debug_span!(
                    parent: None,
                    "http_tracker",
                    tracker = %url,
                    info_hash = ?info_hash
                );
                self.task_single_tracker_monitor_http(url, generation)
                    .instrument(span)
                    .left_future()
            }
        }
    }

    /// Whether the loop for `url` started at `generation` should keep going.
    /// Without a status registry there is no way to remove a tracker.
    fn is_active(&self, url: &Url, generation: Option<u64>) -> bool {
        match (&self.status, generation) {
            (Some(s), Some(generation)) => s.is_current(url.as_str(), generation),
            _ => true,
        }
    }

    /// Sleep for `interval`, waking early on a reannounce request or removal.
    async fn wait(&self, interval: Duration, kick: &mut Option<watch::Receiver<u64>>) {
        match kick {
            Some(kick) => {
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {}
                    changed = kick.changed() => {
                        if changed.is_err() {
                            tokio::time::sleep(interval).await;
                        }
                    }
                }
            }
            None => tokio::time::sleep(interval).await,
        }
    }

    async fn task_single_tracker_monitor_http(
        &self,
        tracker_url: Url,
        generation: Option<u64>,
    ) -> anyhow::Result<()> {
        trace!(url=%tracker_url, "starting monitor");
        let mut first = true;
        let mut was_completed = false;
        let mut kick = self.status.as_ref().map(|s| s.subscribe());

        loop {
            if !self.is_active(&tracker_url, generation) {
                debug!("tracker removed, stopping announces");
                return Ok(());
            }
            let stats = self.stats.get();
            let is_completed = stats.is_completed();

            let event = if first {
                first = false;
                Some(tracker_comms_http::TrackerRequestEvent::Started)
            } else if matches!(stats.torrent_state, TrackerCommsStatsState::Paused) {
                Some(tracker_comms_http::TrackerRequestEvent::Stopped)
            } else if is_completed && !was_completed {
                Some(tracker_comms_http::TrackerRequestEvent::Completed)
            } else {
                None
            };

            was_completed = is_completed;

            if let Some(s) = &self.status {
                s.record_updating(tracker_url.as_str());
            }

            let interval = (|| async {
                if !self.is_active(&tracker_url, generation) {
                    bail!("tracker removed");
                }
                self.tracker_one_request_http(&tracker_url, event).await
            })
            .retry(
                ExponentialBuilder::new()
                    .without_max_times()
                    .with_jitter()
                    .with_factor(2.)
                    .with_min_delay(Duration::from_secs(10))
                    .with_max_delay(Duration::from_secs(600)),
            )
            .when(|_| self.is_active(&tracker_url, generation))
            .notify(|err, retry_in| {
                if let Some(s) = &self.status {
                    s.record_error(tracker_url.as_str(), &format!("{err:#}"));
                }
                debug!(?retry_in, "error calling tracker: {err:#}")
            })
            .await;
            let interval = match interval {
                Ok(interval) => interval,
                Err(_) if !self.is_active(&tracker_url, generation) => {
                    debug!("tracker removed, stopping announces");
                    return Ok(());
                }
                Err(e) => return Err(e).context("this shouldn't fail"),
            };

            let interval = self.force_tracker_interval.unwrap_or(interval);
            debug!("sleeping for {:?} after calling tracker", interval);
            self.wait(interval, &mut kick).await;
        }
    }

    async fn tracker_one_request_http(
        &self,
        tracker_url: &Url,
        event: Option<tracker_comms_http::TrackerRequestEvent>,
    ) -> anyhow::Result<Duration> {
        let stats = self.stats.get();
        let request = tracker_comms_http::TrackerRequest {
            info_hash: &self.info_hash,
            peer_id: &self.peer_id,
            port: self
                .announce_port
                .get()
                .map(std::num::NonZeroU16::get)
                .unwrap_or(4240),
            uploaded: stats.uploaded_bytes,
            downloaded: stats.downloaded_bytes,
            left: stats.get_left_to_download_bytes(),
            compact: true,
            no_peer_id: false,
            event,
            ip: None,
            numwant: None,
            key: Some(self.key),
            trackerid: None,
        };

        let mut url = tracker_url.clone();

        let mut queries = request.as_querystring();
        if let Some(url_query) = url.query() {
            queries.push_str(&format!("&{url_query}"));
        }
        url.set_query(Some(&queries));

        let response: reqwest::Response = self.reqwest_client.get(url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("tracker responded with {:?}", response.status());
        }
        let bytes = response.bytes().await?;
        if let Ok((error, _)) =
            bencode::from_bytes_with_rest::<tracker_comms_http::TrackerError>(&bytes)
        {
            anyhow::bail!(
                "tracker returned failure. Failure reason: {}",
                error.failure_reason
            )
        };
        let response = bencode::from_bytes_with_rest::<tracker_comms_http::TrackerResponse>(&bytes)
            .map_err(|e| {
                tracing::trace!("error deserializing TrackerResponse: {e:#}");
                e.into_kind()
            })?
            .0;

        let mut peers_returned: u32 = 0;
        for peer in response.iter_peers() {
            peers_returned += 1;
            self.tx.send(peer).await?;
        }
        let interval = response.min_interval.unwrap_or(response.interval);
        if let Some(s) = &self.status {
            s.record_success(
                tracker_url.as_str(),
                u32::try_from(response.complete).ok(),
                u32::try_from(response.incomplete).ok(),
                peers_returned,
                interval,
            );
        }
        Ok(Duration::from_secs(interval))
    }

    async fn task_single_tracker_monitor_udp(
        &self,
        url: Url,
        generation: Option<u64>,
        client: UdpTrackerClient,
    ) -> anyhow::Result<()> {
        if url.scheme() != "udp" {
            bail!("expected UDP scheme in {url}");
        }
        let (host, port) = (
            url.host().context("missing host")?,
            url.port().context("missing port")?,
        );

        let mut sleep_interval: Option<Duration> = None;
        let mut prev_addrs: Option<UdpTrackerResolveResult> = None;
        let mut kick = self.status.as_ref().map(|s| s.subscribe());
        loop {
            if let Some(i) = sleep_interval {
                trace!(interval=?sleep_interval, "sleeping");
                self.wait(i, &mut kick).await;
            }
            if !self.is_active(&url, generation) {
                debug!("tracker removed, stopping announces");
                return Ok(());
            }

            if let Some(s) = &self.status {
                s.record_updating(url.as_str());
            }

            // This should retry forever until the addrs are resolved.
            let addrs = (async || {
                udp_tracker_to_socket_addrs(host.clone(), port)
                    .instrument(trace_span!("resolve", ?host))
                    .await
                    .or_else(|err| prev_addrs.ok_or(err))
            })
            .retry(
                ExponentialBuilder::new()
                    .without_max_times()
                    .with_max_delay(Duration::from_secs(60))
                    .with_jitter(),
            )
            .notify(|err, retry| debug!(retry_in=?retry, "error resolving tracker: {err:#}"))
            .await
            .context("this shouldn't happen: failed resolving tracker addrs")?;

            prev_addrs = Some(addrs);

            match addrs {
                UdpTrackerResolveResult::One(addr) => {
                    match self
                        .tracker_one_request_udp(&url, addr, &client)
                        .instrument(trace_span!("udp request", ?addr))
                        .await
                    {
                        Ok(sleep) => sleep_interval = Some(sleep),
                        Err(_) => {
                            sleep_interval = Some(sleep_interval.unwrap_or(Duration::from_secs(60)))
                        }
                    }
                }
                UdpTrackerResolveResult::Two(v4, v6) => {
                    let (r4, r6) = tokio::join!(
                        self.tracker_one_request_udp(&url, v4.into(), &client)
                            .instrument(trace_span!("udp request", addr=?v4)),
                        self.tracker_one_request_udp(&url, v6.into(), &client)
                            .instrument(trace_span!("udp request", addr=?v6))
                    );
                    sleep_interval = Some(
                        r4.or(r6)
                            .ok()
                            .or(sleep_interval)
                            .unwrap_or(Duration::from_secs(60)),
                    )
                }
            }
        }
    }

    async fn tracker_one_request_udp(
        &self,
        url: &Url,
        addr: SocketAddr,
        client: &UdpTrackerClient,
    ) -> anyhow::Result<Duration> {
        use tracker_comms_udp::*;

        let stats = self.stats.get();
        let request = AnnounceFields {
            info_hash: self.info_hash,
            peer_id: self.peer_id,
            downloaded: stats.downloaded_bytes,
            left: stats.get_left_to_download_bytes(),
            uploaded: stats.uploaded_bytes,
            event: match stats.torrent_state {
                TrackerCommsStatsState::None => EVENT_NONE,
                TrackerCommsStatsState::Initializing => EVENT_STARTED,
                TrackerCommsStatsState::Paused => EVENT_STOPPED,
                TrackerCommsStatsState::Live => {
                    if stats.is_completed() {
                        EVENT_COMPLETED
                    } else {
                        EVENT_STARTED
                    }
                }
            },
            key: self.key,
            port: self
                .announce_port
                .get()
                .map(std::num::NonZeroU16::get)
                .unwrap_or(4240),
        };

        match client.announce(addr, request).await {
            Ok(response) => {
                trace!(len = response.addrs.len(), "received announce response");
                let peers_returned = response.addrs.len() as u32;
                for addr in response.addrs {
                    self.tx.send(addr).await.context("rx closed")?;
                }
                let sleep = response.interval.max(5) as u64;
                if let Some(s) = &self.status {
                    s.record_success(
                        url.as_str(),
                        Some(response.seeders),
                        Some(response.leechers),
                        peers_returned,
                        sleep,
                    );
                }
                Ok(Duration::from_secs(sleep))
            }
            Err(e) => {
                if let Some(s) = &self.status {
                    s.record_error(url.as_str(), &format!("{e:#}"));
                }
                debug!(?addr, "error reading announce response: {e:#}");
                Err(e)
            }
        }
    }
}
