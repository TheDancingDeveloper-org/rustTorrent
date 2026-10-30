use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use librtbit_core::Id20;
use parking_lot::RwLock;
use tokio::{net::UdpSocket, task::JoinHandle};
use tokio_util::sync::CancellationToken;

const ACTION_CONNECT: u32 = 0;
const ACTION_ANNOUNCE: u32 = 1;
const ACTION_SCRAPE: u32 = 2;
const ACTION_ERROR: u32 = 3;
const CONNECTION_ID: u64 = 0x0102_0304_0506_0708;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrackerStats {
    pub connects: usize,
    pub announces: usize,
    pub scrapes: usize,
}

#[derive(Default)]
struct MockTrackerState {
    peers: RwLock<HashMap<Id20, Vec<TrackedPeer>>>,
    connects: AtomicUsize,
    announces: AtomicUsize,
    scrapes: AtomicUsize,
}

#[derive(Clone, Copy)]
struct TrackedPeer {
    addr: SocketAddr,
    is_seed: bool,
}

pub struct MockUdpTracker {
    addr: SocketAddr,
    state: Arc<MockTrackerState>,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl MockUdpTracker {
    pub async fn start() -> Self {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = socket.local_addr().unwrap();
        let state = Arc::new(MockTrackerState::default());
        let cancel = CancellationToken::new();
        let task = tokio::spawn(run_tracker(socket, state.clone(), cancel.clone()));
        Self {
            addr,
            state,
            cancel,
            task,
        }
    }

    pub fn url(&self) -> String {
        format!("udp://{}/announce", self.addr)
    }

    pub fn stats(&self) -> TrackerStats {
        TrackerStats {
            connects: self.state.connects.load(Ordering::Relaxed),
            announces: self.state.announces.load(Ordering::Relaxed),
            scrapes: self.state.scrapes.load(Ordering::Relaxed),
        }
    }

    pub fn peers(&self, info_hash: Id20) -> Vec<SocketAddr> {
        self.state
            .peers
            .read()
            .get(&info_hash)
            .map(|peers| peers.iter().map(|peer| peer.addr).collect())
            .unwrap_or_default()
    }

    pub async fn shutdown(self) {
        self.cancel.cancel();
        self.task.await.unwrap();
    }
}

async fn run_tracker(socket: UdpSocket, state: Arc<MockTrackerState>, cancel: CancellationToken) {
    let mut buf = [0u8; 2048];
    loop {
        let result = tokio::select! {
            () = cancel.cancelled() => return,
            result = socket.recv_from(&mut buf) => result,
        };
        let Ok((len, source)) = result else { continue };
        if len < 16 {
            continue;
        }
        let connection_id = u64::from_be_bytes(buf[0..8].try_into().unwrap());
        let action = u32::from_be_bytes(buf[8..12].try_into().unwrap());
        let tid = &buf[12..16];
        let mut response = Vec::new();

        match action {
            ACTION_CONNECT => {
                state.connects.fetch_add(1, Ordering::Relaxed);
                response.extend_from_slice(&ACTION_CONNECT.to_be_bytes());
                response.extend_from_slice(tid);
                response.extend_from_slice(&CONNECTION_ID.to_be_bytes());
            }
            ACTION_ANNOUNCE if connection_id == CONNECTION_ID && len >= 98 => {
                state.announces.fetch_add(1, Ordering::Relaxed);
                let info_hash = Id20::new(buf[16..36].try_into().unwrap());
                let left = u64::from_be_bytes(buf[64..72].try_into().unwrap());
                let port = u16::from_be_bytes(buf[96..98].try_into().unwrap());
                let peer = SocketAddr::new(source.ip(), port);
                let peers = {
                    let mut all = state.peers.write();
                    let peers = all.entry(info_hash).or_default();
                    if let Some(existing) = peers.iter_mut().find(|existing| existing.addr == peer)
                    {
                        existing.is_seed = left == 0;
                    } else {
                        peers.push(TrackedPeer {
                            addr: peer,
                            is_seed: left == 0,
                        });
                    }
                    peers.clone()
                };
                // A test swarm never approaches u32::MAX peers, so these cannot fail.
                let seeders = u32::try_from(peers.iter().filter(|peer| peer.is_seed).count())
                    .expect("peer count fits in u32");
                let leechers =
                    u32::try_from(peers.len()).expect("peer count fits in u32") - seeders;
                response.extend_from_slice(&ACTION_ANNOUNCE.to_be_bytes());
                response.extend_from_slice(tid);
                response.extend_from_slice(&1u32.to_be_bytes());
                response.extend_from_slice(&leechers.to_be_bytes());
                response.extend_from_slice(&seeders.to_be_bytes());
                for tracked in peers.into_iter().filter(|tracked| tracked.addr != peer) {
                    if let SocketAddr::V4(addr) = tracked.addr {
                        response.extend_from_slice(&addr.ip().octets());
                        response.extend_from_slice(&addr.port().to_be_bytes());
                    }
                }
            }
            ACTION_SCRAPE if connection_id == CONNECTION_ID && len >= 36 => {
                state.scrapes.fetch_add(1, Ordering::Relaxed);
                response.extend_from_slice(&ACTION_SCRAPE.to_be_bytes());
                response.extend_from_slice(tid);
                let (hashes, _) = buf[16..len].as_chunks::<20>();
                for hash in hashes {
                    let hash = Id20::new(*hash);
                    let peers = state.peers.read();
                    let (seeders, leechers) = peers.get(&hash).map_or((0, 0), |peers| {
                        let seeders =
                            u32::try_from(peers.iter().filter(|peer| peer.is_seed).count())
                                .expect("peer count fits in u32");
                        (
                            seeders,
                            u32::try_from(peers.len()).expect("peer count fits in u32") - seeders,
                        )
                    });
                    response.extend_from_slice(&seeders.to_be_bytes());
                    response.extend_from_slice(&0u32.to_be_bytes());
                    response.extend_from_slice(&leechers.to_be_bytes());
                }
            }
            _ => {
                response.extend_from_slice(&ACTION_ERROR.to_be_bytes());
                response.extend_from_slice(tid);
                response.extend_from_slice(b"invalid request");
            }
        }
        let _ = socket.send_to(&response, source).await;
    }
}

#[cfg(test)]
mod tests {
    use std::{net::Ipv4Addr, time::Duration};

    use librtbit_core::Id20;
    use tracker_comms::{AnnounceFields, UdpTrackerClient};

    use crate::{
        AddTorrent, AddTorrentOptions, CreateTorrentOptions, ListenerMode, Session, SessionOptions,
        create_torrent,
        listen::ListenerOptions,
        spawn_utils::BlockingSpawner,
        tests::test_util::{create_new_file_with_random_content, wait_until},
    };

    use super::*;

    fn available_port() -> u16 {
        std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    #[tokio::test]
    async fn supports_connect_announce_and_scrape() {
        let tracker = MockUdpTracker::start().await;
        let client = UdpTrackerClient::new(CancellationToken::new(), None)
            .await
            .unwrap();
        let hash = Id20::new([7; 20]);
        let announced = client
            .announce(
                tracker.addr,
                AnnounceFields {
                    info_hash: hash,
                    peer_id: Id20::new([8; 20]),
                    downloaded: 0,
                    left: 10,
                    uploaded: 0,
                    event: 2,
                    key: 1,
                    port: 6881,
                },
            )
            .await
            .unwrap();
        assert!(announced.addrs.is_empty());
        assert_eq!(tracker.peers(hash).len(), 1);

        let scrape = client.scrape(tracker.addr, vec![hash]).await.unwrap();
        assert_eq!(scrape.len(), 1);
        assert_eq!(scrape[0].seeders, 0);
        assert_eq!(scrape[0].leechers, 1);
        assert_eq!(
            tracker.stats(),
            TrackerStats {
                connects: 1,
                announces: 1,
                scrapes: 1
            }
        );
        tracker.shutdown().await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn live_tracker_worker_observes_announce_port_update() {
        use std::num::NonZeroU16;

        let tracker = MockUdpTracker::start().await;
        let output = tempfile::TempDir::with_prefix("tracker_port_update").unwrap();
        let source = output.path().join("payload.bin");
        create_new_file_with_random_content(&source, 16 * 1024);
        let torrent = create_torrent(
            &source,
            CreateTorrentOptions {
                trackers: vec![tracker.url()],
                piece_length: Some(16 * 1024),
                ..Default::default()
            },
            &BlockingSpawner::new(1),
        )
        .await
        .unwrap();
        let info_hash = torrent.info_hash();
        let listen_port = available_port();
        let initial_announce_port = available_port();
        let updated_announce_port = available_port();
        let session = Session::new_with_opts(
            output.path().to_owned(),
            SessionOptions {
                disable_dht: true,
                disable_local_service_discovery: true,
                listen: Some(ListenerOptions {
                    mode: ListenerMode::TcpOnly,
                    listen_addr: (Ipv4Addr::LOCALHOST, listen_port).into(),
                    announce_port: Some(initial_announce_port),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let handle = session
            .add_torrent(
                AddTorrent::TorrentFileBytes(torrent.as_bytes().unwrap()),
                Some(AddTorrentOptions {
                    overwrite: true,
                    force_tracker_interval: Some(Duration::from_secs(1)),
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();

        wait_until(
            || {
                let peers = tracker.peers(info_hash);
                if peers
                    .iter()
                    .any(|peer| peer.port() == initial_announce_port)
                {
                    Ok(())
                } else {
                    anyhow::bail!("initial announce port not observed: {peers:?}")
                }
            },
            Duration::from_secs(10),
        )
        .await
        .unwrap();

        session.set_announce_port(NonZeroU16::new(updated_announce_port).unwrap());

        wait_until(
            || {
                let peers = tracker.peers(info_hash);
                if peers
                    .iter()
                    .any(|peer| peer.port() == updated_announce_port)
                {
                    Ok(())
                } else {
                    anyhow::bail!("updated announce port not observed: {peers:?}")
                }
            },
            Duration::from_secs(10),
        )
        .await
        .unwrap();
        assert_eq!(session.listen_addr().unwrap().port(), listen_port);
        assert_eq!(session.announce_port(), Some(updated_announce_port));

        drop(handle);
        drop(session);
        tracker.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn leech_discovers_seed_through_tracker() {
        let tracker = MockUdpTracker::start().await;
        let seed_root = tempfile::TempDir::with_prefix("tracker_seed").unwrap();
        let source = seed_root.path().join("payload.bin");
        create_new_file_with_random_content(&source, 256 * 1024);
        let torrent = create_torrent(
            &source,
            CreateTorrentOptions {
                trackers: vec![tracker.url()],
                piece_length: Some(16 * 1024),
                ..Default::default()
            },
            &BlockingSpawner::new(1),
        )
        .await
        .unwrap();
        let torrent_bytes = torrent.as_bytes().unwrap();
        let seed_port = available_port();
        let leech_port = available_port();

        let seed = Session::new_with_opts(
            seed_root.path().to_owned(),
            SessionOptions {
                disable_dht: true,
                disable_local_service_discovery: true,
                listen: Some(ListenerOptions {
                    mode: ListenerMode::TcpOnly,
                    listen_addr: (Ipv4Addr::LOCALHOST, seed_port).into(),
                    announce_port: Some(seed_port),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let seed_handle = seed
            .add_torrent(
                AddTorrent::TorrentFileBytes(torrent_bytes.clone()),
                Some(AddTorrentOptions {
                    overwrite: true,
                    force_tracker_interval: Some(Duration::from_secs(1)),
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();
        wait_until(
            || {
                if seed_handle.stats().finished {
                    Ok(())
                } else {
                    anyhow::bail!("seed is not ready")
                }
            },
            Duration::from_secs(10),
        )
        .await
        .unwrap();
        wait_until(
            || {
                if tracker.peers(torrent.info_hash()).is_empty() {
                    anyhow::bail!("seed has not announced")
                }
                Ok(())
            },
            Duration::from_secs(10),
        )
        .await
        .unwrap();

        let leech_root = tempfile::TempDir::with_prefix("tracker_leech").unwrap();
        let leech = Session::new_with_opts(
            leech_root.path().to_owned(),
            SessionOptions {
                disable_dht: true,
                disable_local_service_discovery: true,
                listen: Some(ListenerOptions {
                    mode: ListenerMode::TcpOnly,
                    listen_addr: (Ipv4Addr::LOCALHOST, leech_port).into(),
                    announce_port: Some(leech_port),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let leech_handle = leech
            .add_torrent(
                AddTorrent::TorrentFileBytes(torrent_bytes),
                Some(AddTorrentOptions {
                    overwrite: true,
                    force_tracker_interval: Some(Duration::from_secs(1)),
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();
        if tokio::time::timeout(Duration::from_secs(20), leech_handle.wait_until_completed())
            .await
            .is_err()
        {
            panic!(
                "tracker-discovered download timed out: tracker={:?}, seed={:?}, leech={:?}",
                tracker.peers(torrent.info_hash()),
                seed_handle.stats(),
                leech_handle.stats(),
            );
        }

        let downloaded = leech_root.path().join("payload.bin");
        assert_eq!(
            std::fs::read(&source).unwrap(),
            std::fs::read(downloaded).unwrap()
        );
        assert!(tracker.stats().announces >= 2);

        drop(leech_handle);
        drop(leech);
        drop(seed_handle);
        drop(seed);
        tracker.shutdown().await;
    }
}
