use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{Context, bail};
use librqbit_dualstack_sockets::ConnectOpts;
use librqbit_utp::{BindDevice, UtpSocketUdp};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tracing::debug;

use crate::{
    Error, PeerConnectionOptions, Result,
    type_aliases::{BoxAsyncReadVectored, BoxAsyncWrite},
    vectored_traits::AsyncReadVectoredIntoCompat,
};

#[derive(Debug, Clone, Copy, Serialize)]
pub enum ConnectionKind {
    #[serde(rename = "tcp")]
    Tcp,
    #[serde(rename = "utp")]
    Utp,
    #[serde(rename = "socks")]
    Socks,
}

impl std::fmt::Display for ConnectionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectionKind::Tcp => f.write_str("tcp"),
            ConnectionKind::Utp => f.write_str("uTP"),
            ConnectionKind::Socks => f.write_str("socks"),
        }
    }
}

pub struct ConnectionOptions {
    // socks5://[username:password@]host:port
    // If set, all outgoing connections will go through the proxy over TCP.
    pub proxy_url: Option<String>,
    // TCP outgoing connections are enabled by default
    pub enable_tcp: bool,
    pub peer_opts: Option<PeerConnectionOptions>,
}

impl Default for ConnectionOptions {
    fn default() -> Self {
        Self {
            enable_tcp: true,
            proxy_url: None,
            peer_opts: None,
        }
    }
}

/// Outgoing proxy. Credentials stay on this type and are never serialized.
#[derive(Clone)]
pub(crate) enum ProxyConfig {
    Socks5(SocksProxyConfig),
    Http {
        host: String,
        port: u16,
        username: Option<String>,
        password: Option<String>,
    },
}

impl std::fmt::Debug for ProxyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print a password. `redacted` keeps the user and masks the secret.
        f.write_str(&self.redacted())
    }
}

impl ProxyConfig {
    /// `socks5://` or `http://` / `https://`, optional userinfo. The password is kept
    /// and is not included in [`Self::redacted`].
    #[allow(dead_code)]
    pub fn parse(url: &str) -> anyhow::Result<Self> {
        let parsed = ::url::Url::parse(url).context("invalid proxy URL")?;
        let host = parsed.host_str().context("missing proxy host")?.to_owned();
        let port = parsed.port().context("missing proxy port")?;
        let username = if parsed.username().is_empty() {
            None
        } else {
            Some(parsed.username().to_owned())
        };
        let password = parsed.password().map(|p| p.to_owned());
        match parsed.scheme() {
            "socks5" => Ok(Self::Socks5(SocksProxyConfig {
                host,
                port,
                username_password: match (username, password) {
                    (Some(u), Some(p)) => Some((u, p)),
                    (Some(u), None) => Some((u, String::new())),
                    _ => None,
                },
            })),
            "http" | "https" => Ok(Self::Http {
                host,
                port,
                username,
                password,
            }),
            other => anyhow::bail!("proxy scheme {other} is not supported; use socks5 or http"),
        }
    }

    /// URL safe to log or return from the API: user is kept, password is `***`.
    pub fn redacted(&self) -> String {
        match self {
            Self::Socks5(s) => match &s.username_password {
                Some((u, _)) => format!("socks5://{u}:***@{}:{}", s.host, s.port),
                None => format!("socks5://{}:{}", s.host, s.port),
            },
            Self::Http {
                host,
                port,
                username,
                password,
            } => {
                let scheme = "http";
                match (username, password) {
                    (Some(u), Some(_)) => format!("{scheme}://{u}:***@{host}:{port}"),
                    (Some(u), None) => format!("{scheme}://{u}@{host}:{port}"),
                    _ => format!("{scheme}://{host}:{port}"),
                }
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct SocksProxyConfig {
    pub host: String,
    pub port: u16,
    pub username_password: Option<(String, String)>,
}

impl std::fmt::Debug for SocksProxyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let user = self
            .username_password
            .as_ref()
            .map(|(u, _)| format!("{u}:***"));
        f.debug_struct("SocksProxyConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username_password", &user)
            .finish()
    }
}
#[derive(Default, Clone)]
pub(crate) struct StreamConnectorArgs {
    pub enable_tcp: bool,
    pub proxy_config: Option<ProxyConfig>,
    pub utp_socket: Option<Arc<UtpSocketUdp>>,
    pub bind_device: Option<BindDevice>,
    pub ipv4_only: bool,
}

impl std::fmt::Debug for StreamConnectorArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamConnectorArgs")
            .field("enable_tcp", &self.enable_tcp)
            .field(
                "proxy_config",
                &self.proxy_config.as_ref().map(|p| p.redacted()),
            )
            .field(
                "utp_socket",
                &self.utp_socket.as_ref().map(|_| "UtpSocketUdp"),
            )
            .field("bind_device", &self.bind_device)
            .field("ipv4_only", &self.ipv4_only)
            .finish()
    }
}

impl ProxyConfig {
    pub(crate) async fn connect(
        &self,
        addr: SocketAddr,
    ) -> anyhow::Result<(
        Box<dyn tokio::io::AsyncRead + Unpin + Send>,
        Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
    )> {
        match self {
            Self::Socks5(cfg) => {
                let (r, w) = cfg.connect(addr).await.context("SOCKS5 connect")?;
                Ok((Box::new(r), Box::new(w)))
            }
            Self::Http {
                host,
                port,
                username,
                password,
            } => http_connect(host, *port, username.as_deref(), password.as_deref(), addr).await,
        }
    }
}

/// Read one proxy reply line, refusing anything longer than `max` bytes.
async fn read_proxy_line<R>(reader: &mut R, buf: &mut String, max: usize) -> std::io::Result<usize>
where
    R: AsyncBufReadExt + Unpin,
{
    let n = reader.take(max as u64 + 1).read_line(buf).await?;
    if n > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "HTTP proxy reply line is too long",
        ));
    }
    Ok(n)
}

async fn http_connect(
    host: &str,
    port: u16,
    username: Option<&str>,
    password: Option<&str>,
    addr: SocketAddr,
) -> anyhow::Result<(
    Box<dyn tokio::io::AsyncRead + Unpin + Send>,
    Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
)> {
    let mut stream = tokio::net::TcpStream::connect((host, port))
        .await
        .context("connecting to HTTP proxy")?;
    let mut request =
        format!("CONNECT {addr} HTTP/1.1\r\nHost: {addr}\r\nProxy-Connection: keep-alive\r\n");
    if let Some(user) = username {
        use base64::Engine;
        let raw = format!("{user}:{}", password.unwrap_or(""));
        let token = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
        request.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .context("writing HTTP CONNECT")?;
    let mut reader = BufReader::new(stream);
    // A hostile proxy must not be able to grow these without bound.
    const MAX_PROXY_LINE: usize = 8 * 1024;
    const MAX_PROXY_HEADERS: usize = 64;
    let mut status = String::new();
    read_proxy_line(&mut reader, &mut status, MAX_PROXY_LINE)
        .await
        .context("reading HTTP CONNECT status")?;
    let code = status.split_whitespace().nth(1).unwrap_or("");
    if code != "200" {
        let shown = status.trim();
        anyhow::bail!("HTTP proxy CONNECT was refused ({shown})");
    }
    let mut finished_headers = false;
    for _ in 0..MAX_PROXY_HEADERS {
        let mut line = String::new();
        let n = read_proxy_line(&mut reader, &mut line, MAX_PROXY_LINE)
            .await
            .context("reading HTTP CONNECT headers")?;
        if n == 0 || line == "\r\n" || line == "\n" {
            finished_headers = true;
            break;
        }
    }
    if !finished_headers {
        anyhow::bail!("HTTP proxy sent too many header lines");
    }
    let stream = reader.into_inner();
    let (r, w) = tokio::io::split(stream);
    Ok((Box::new(r), Box::new(w)))
}

impl SocksProxyConfig {
    async fn connect(
        &self,
        addr: SocketAddr,
    ) -> tokio_socks::Result<(
        impl tokio::io::AsyncRead + Unpin + 'static,
        impl tokio::io::AsyncWrite + Unpin + 'static,
    )> {
        let proxy_addr = (self.host.as_str(), self.port);

        let stream = if let Some((username, password)) = self.username_password.as_ref() {
            tokio_socks::tcp::Socks5Stream::connect_with_password(
                proxy_addr,
                addr,
                username.as_str(),
                password.as_str(),
            )
            .await?
        } else {
            tokio_socks::tcp::Socks5Stream::connect(proxy_addr, addr).await?
        };

        Ok(tokio::io::split(stream))
    }
}

gen_stats!(SingleStatAtomic SingleStatSnapshot, [
    attempts u64,
    successes u64,
    errors u64
], []);
gen_stats!(PerFamilyAtomic PerFamilySnapshot, [], [
    v4 SingleStatAtomic SingleStatSnapshot,
    v6 SingleStatAtomic SingleStatSnapshot
]);
gen_stats!(ConnectStatsAtomic ConnectStatsSnapshot, [], [
    socks PerFamilyAtomic PerFamilySnapshot,
    tcp PerFamilyAtomic PerFamilySnapshot,
    utp PerFamilyAtomic PerFamilySnapshot
]);

pub(crate) struct StreamConnector {
    proxy_config: parking_lot::RwLock<Option<ProxyConfig>>,
    enable_tcp: bool,
    bind_device: Option<BindDevice>,
    utp_socket: Option<Arc<librqbit_utp::UtpSocketUdp>>,
    stats: ConnectStatsAtomic,
    ipv4_only: bool,
}

impl std::fmt::Debug for StreamConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let proxy = self.proxy_config.read().as_ref().map(|p| p.redacted());
        f.debug_struct("StreamConnector")
            .field("proxy_config", &proxy)
            .field("enable_tcp", &self.enable_tcp)
            .field("ipv4_only", &self.ipv4_only)
            .finish()
    }
}

impl StreamConnector {
    pub async fn new(config: StreamConnectorArgs) -> anyhow::Result<Self> {
        #[allow(clippy::single_match)]
        match (
            config.proxy_config.is_some(),
            config.enable_tcp,
            config.utp_socket.is_some(),
        ) {
            (false, false, false) => {
                bail!("no way to connect to peers, enable TCP, uTP or socks proxy")
            }
            _ => {
                // TODO: maybe validate other combinations. For now there's no way to disable TCP
            }
        }

        Ok(Self {
            proxy_config: parking_lot::RwLock::new(config.proxy_config),
            enable_tcp: config.enable_tcp,
            utp_socket: config.utp_socket,
            bind_device: config.bind_device,
            stats: Default::default(),
            ipv4_only: config.ipv4_only,
        })
    }

    fn get_stat(&self, kind: ConnectionKind, is_v6: bool) -> &SingleStatAtomic {
        let stat = match kind {
            ConnectionKind::Tcp => &self.stats.tcp,
            ConnectionKind::Utp => &self.stats.utp,
            ConnectionKind::Socks => &self.stats.socks,
        };
        if is_v6 { &stat.v6 } else { &stat.v4 }
    }

    async fn with_stat<R, E>(
        &self,
        kind: ConnectionKind,
        is_v6: bool,
        fut: impl Future<Output = std::result::Result<R, E>>,
    ) -> std::result::Result<R, E> {
        let stat = self.get_stat(kind, is_v6);
        stat.attempts(1);
        fut.await
            .inspect(|_| stat.successes(1))
            .inspect_err(|_| stat.errors(1))
    }

    async fn tcp_connect(
        &self,
        addr: SocketAddr,
    ) -> librqbit_dualstack_sockets::Result<tokio::net::TcpStream> {
        self.with_stat(
            ConnectionKind::Tcp,
            addr.is_ipv6(),
            librqbit_dualstack_sockets::tcp_connect(
                addr,
                ConnectOpts {
                    // Setting source port doesn't work with cloudflare warp on linux
                    // source_port: self.tcp_source_port,
                    source_port: None,
                    bind_device: self.bind_device.as_ref(),
                },
            ),
        )
        .await
    }

    pub fn stats(&self) -> &ConnectStatsAtomic {
        &self.stats
    }

    pub(crate) fn set_proxy(&self, proxy: Option<ProxyConfig>) {
        *self.proxy_config.write() = proxy;
    }

    pub async fn connect(
        &self,
        addr: SocketAddr,
    ) -> Result<(ConnectionKind, BoxAsyncReadVectored, BoxAsyncWrite)> {
        if addr.port() == 0 {
            return Err(Error::Anyhow(anyhow::anyhow!(
                "invalid peer address (port 0): {addr}"
            )));
        }

        if self.ipv4_only && addr.is_ipv6() {
            return Err(Error::Anyhow(anyhow::anyhow!(
                "ipv6 disabled, skipping connection to {addr}"
            )));
        }

        let proxy = self.proxy_config.read().clone();
        if let Some(proxy) = proxy.as_ref() {
            let (r, w) = self
                .with_stat(ConnectionKind::Socks, addr.is_ipv6(), proxy.connect(addr))
                .await
                .map_err(Error::Anyhow)?;
            debug!(?addr, "connected through proxy");
            return Ok((
                ConnectionKind::Socks,
                Box::new(r.into_vectored_compat()),
                Box::new(w),
            ));
        }

        // Try to connect over TCP first. If in 1 second we haven't connected, try uTP also (if configured).
        // Whoever connects first wins.
        let tcp_connect = async {
            if !self.enable_tcp {
                return Ok(None);
            }
            let conn = self.tcp_connect(addr).await?;
            debug!(?addr, "connected over TCP");
            Ok::<_, librqbit_dualstack_sockets::Error>(Some(conn))
        };

        let tcp_failed_notify = tokio::sync::Notify::new();

        let utp_connect = async {
            let sock = match self.utp_socket.as_ref() {
                Some(sock) => sock,
                None => return Ok(None),
            };

            // Give TCP priority as it's more mature and simpler.
            if self.enable_tcp {
                // wait until either 1 second has passed or TCP failed.
                tokio::select! {
                    _ = tcp_failed_notify.notified() => {},
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                }
            }

            let conn = self
                .with_stat(ConnectionKind::Utp, addr.is_ipv6(), sock.connect(addr))
                .await?;

            debug!(?addr, "connected over uTP");
            Ok(Some(conn))
        };

        tokio::pin!(tcp_connect);
        tokio::pin!(utp_connect);

        let mut tcp_err: Option<Option<librqbit_dualstack_sockets::Error>> = None;
        let mut utp_err: Option<Option<librqbit_utp::Error>> = None;

        // wait until all fail, or one succeeds.
        loop {
            if let (Some(tcp), Some(utp)) = (tcp_err.as_mut(), utp_err.as_mut()) {
                match (tcp.take(), utp.take()) {
                    (Some(tcp), Some(utp)) => return Err(Error::Connect { tcp, utp }),
                    (Some(tcp), None) => return Err(Error::TcpConnect(tcp)),
                    (None, Some(utp)) => return Err(Error::UtpConnect(utp)),
                    (None, None) => return Err(Error::ConnectDisabled),
                }
            }
            tokio::select! {
                tcp_res = &mut tcp_connect, if tcp_err.is_none() => {
                    match tcp_res {
                        Ok(Some(stream)) => {
                            let (r, w) = stream.into_split();
                            return Ok((ConnectionKind::Tcp, Box::new(r), Box::new(w)));
                        },
                        Ok(None) => {
                            tcp_err = Some(None);
                            tcp_failed_notify.notify_waiters();
                        }
                        Err(e) => {
                            tcp_err = Some(Some(e));
                            tcp_failed_notify.notify_waiters();
                        }
                    }
                },
                utp_res = &mut utp_connect, if utp_err.is_none() => {
                    match utp_res {
                        Ok(Some(stream)) => {
                            let (r, w) = stream.split();
                            return Ok((ConnectionKind::Utp, Box::new(r), Box::new(w)));
                        },
                        Ok(None) => {
                            utp_err = Some(None);
                        }
                        Err(e) => {
                            utp_err = Some(Some(e));
                        }
                    }
                },
            };
        }
    }
}

#[cfg(test)]
mod proxy_config_tests {
    use super::ProxyConfig;

    #[test]
    fn proxy_config_round_trip_masks_the_password() {
        let socks = ProxyConfig::parse("socks5://alice:s3cret@127.0.0.1:1080").unwrap();
        let shown = socks.redacted();
        assert_eq!(shown, "socks5://alice:***@127.0.0.1:1080");
        assert!(!shown.contains("s3cret"));
        let again = ProxyConfig::parse(&shown.replace("***", "s3cret")).unwrap();
        assert_eq!(again.redacted(), shown);

        let http = ProxyConfig::parse("http://bob:hunter2@proxy.example:8080").unwrap();
        let shown = http.redacted();
        assert_eq!(shown, "http://bob:***@proxy.example:8080");
        assert!(!shown.contains("hunter2"));

        let plain = ProxyConfig::parse("http://proxy.example:8080").unwrap();
        assert_eq!(plain.redacted(), "http://proxy.example:8080");

        assert!(ProxyConfig::parse("ftp://proxy.example:21").is_err());
        assert!(ProxyConfig::parse("socks5://noport").is_err());
    }

    #[test]
    fn debug_masks_proxy_credentials() {
        let password = "s3cret-debug";
        let socks =
            ProxyConfig::parse(&format!("socks5://alice:{password}@127.0.0.1:1080")).unwrap();
        let shown = format!("{socks:?}");
        assert!(!shown.contains(password), "{shown}");
        assert!(shown.contains("alice:***"));

        let http =
            ProxyConfig::parse(&format!("http://bob:{password}@proxy.example:8080")).unwrap();
        let shown = format!("{http:?}");
        assert!(!shown.contains(password), "{shown}");

        let args = super::StreamConnectorArgs {
            proxy_config: Some(http.clone()),
            ..Default::default()
        };
        let shown = format!("{args:?}");
        assert!(!shown.contains(password), "{shown}");

        let connector = super::StreamConnector {
            proxy_config: parking_lot::RwLock::new(Some(http)),
            enable_tcp: true,
            bind_device: None,
            utp_socket: None,
            stats: Default::default(),
            ipv4_only: false,
        };
        let shown = format!("{connector:?}");
        assert!(!shown.contains(password), "{shown}");
    }
}
