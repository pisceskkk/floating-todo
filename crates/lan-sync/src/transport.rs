//! Transport abstraction for LAN (HTTP/IPv4) and cross-network (Iroh QUIC) sync.
use iroh::{EndpointId, RelayUrl};
use serde_json::Value;
#[cfg(test)]
use std::net::SocketAddr;
use std::{
    future::Future,
    net::IpAddr,
    pin::Pin,
    str::FromStr,
    sync::Arc,
    time::Duration,
};

pub const MAX_BODY: usize = 512 * 1024;
pub type Result<T> = std::result::Result<T, String>;
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerEndpoint {
    Http(String),
    Iroh {
        id: EndpointId,
        relay: Option<RelayUrl>,
        #[cfg(test)]
        direct: Option<SocketAddr>,
    },
}

impl PeerEndpoint {
    pub fn canonical_url(&self) -> String {
        match self {
            PeerEndpoint::Http(url) => url.clone(),
            PeerEndpoint::Iroh {
                id,
                relay,
                #[cfg(test)]
                direct,
            } => {
                let mut url = format!("iroh://{id}");
                let mut sep = '?';
                if let Some(r) = relay {
                    url.push(sep);
                    url.push_str("relay=");
                    url.push_str(r.as_str().trim_end_matches('/'));
                    sep = '&';
                }
                #[cfg(test)]
                if let Some(addr) = direct {
                    url.push(sep);
                    url.push_str("addr=");
                    url.push_str(&addr.to_string());
                }
                let _ = sep;
                url
            }
        }
    }
}

/// Validate a custom self-hosted iroh-relay URL.
pub fn parse_relay_url(input: &str) -> Result<RelayUrl> {
    let trimmed = input.trim();
    let parsed = reqwest::Url::parse(trimmed)
        .map_err(|_| "自定义中继地址格式无效，请输入 https://域名".to_string())?;
    let scheme_ok = parsed.scheme() == "https"
        || (cfg!(test)
            && parsed.scheme() == "http"
            && matches!(parsed.host_str(), Some("127.0.0.1" | "localhost")));
    if !scheme_ok
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("自定义中继地址仅支持 https:// 网址且不能包含凭据或查询参数".into());
    }
    if parsed.host_str().is_none() {
        return Err("自定义中继地址缺少主机名".into());
    }
    RelayUrl::from_str(trimmed).map_err(|e| format!("无效的中继地址：{e}"))
}

/// Parse and validate either a LAN/Tailscale HTTP endpoint or an Iroh cross-network endpoint.
pub fn parse_endpoint(input: &str) -> Result<PeerEndpoint> {
    let trimmed = input.trim();
    if trimmed.starts_with("iroh://") {
        return parse_iroh_endpoint(trimmed);
    }
    let u = reqwest::Url::parse(trimmed)
        .map_err(|_| "请输入 http://局域网IP:端口 或 iroh://节点地址")?;
    if u.scheme() != "http"
        || !u.username().is_empty()
        || u.password().is_some()
        || u.path() != "/"
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return Err("仅支持局域网 HTTP 地址或 iroh:// 跨网地址".into());
    }
    let ip: IpAddr = u
        .host_str()
        .ok_or("缺少 IP")?
        .trim_matches(['[', ']'])
        .parse()
        .map_err(|_| "请使用 IP 地址")?;
    let allowed = match ip {
        IpAddr::V4(x) => {
            x.is_private()
                || x.is_link_local()
                || (x.octets()[0] == 100 && (64..=127).contains(&x.octets()[1]))
                || cfg!(test) && x.is_loopback()
        }
        IpAddr::V6(_) => false,
    };
    if !allowed {
        return Err("仅允许局域网或虚拟局域网地址".into());
    }
    Ok(PeerEndpoint::Http(trimmed.trim_end_matches('/').to_owned()))
}

fn parse_iroh_endpoint(input: &str) -> Result<PeerEndpoint> {
    let rest = input
        .strip_prefix("iroh://")
        .ok_or_else(|| "无效的 iroh 地址".to_string())?;
    let (host_part, query_part) = match rest.split_once('?') {
        Some((h, q)) => (h, Some(q)),
        None => (rest, None),
    };
    let host = host_part.trim_end_matches('/');
    if host.is_empty() || host.contains('/') || host.contains('@') || host.contains('#') {
        return Err("无效的 iroh 节点 ID".into());
    }
    let id = EndpointId::from_str(host).map_err(|_| "无效的 iroh 节点 ID".to_string())?;
    let mut relay = None;
    #[cfg(test)]
    let mut direct = None;
    if let Some(q) = query_part {
        if q.contains('#') {
            return Err("iroh 地址不能包含片段标识".into());
        }
        for pair in q.split('&').filter(|s| !s.is_empty()) {
            let (k, v) = pair
                .split_once('=')
                .ok_or_else(|| "无效的 iroh 地址参数".to_string())?;
            match k {
                "relay" => {
                    if relay.is_some() {
                        return Err("重复的 relay 参数".into());
                    }
                    relay = Some(parse_relay_url(v)?);
                }
                #[cfg(test)]
                "addr" => {
                    let addr: SocketAddr = v
                        .parse()
                        .map_err(|_| "无效的测试直连地址".to_string())?;
                    if !addr.ip().is_loopback() {
                        return Err("测试直连地址仅允许回环地址".into());
                    }
                    direct = Some(addr);
                }
                _ => return Err(format!("不支持的 iroh 地址参数：{k}")),
            }
        }
    }
    Ok(PeerEndpoint::Iroh {
        id,
        relay,
        #[cfg(test)]
        direct,
    })
}

pub fn endpoint(input: &str) -> Result<String> {
    Ok(parse_endpoint(input)?.canonical_url())
}

/// Transport-agnostic RPC client interface.
pub trait Transport: Send + Sync {
    fn supports(&self, target: &PeerEndpoint) -> bool;
    fn request<'a>(
        &'a self,
        target: &'a PeerEndpoint,
        path: &'a str,
        body: Option<Value>,
    ) -> BoxFuture<'a, Result<Value>>;
}

/// HTTP/IPv4 transport used for local LAN and Tailscale direct sync.
pub struct HttpTransport {
    client: reqwest::Client,
}

impl HttpTransport {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(4))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { client })
    }
}

impl Transport for HttpTransport {
    fn supports(&self, target: &PeerEndpoint) -> bool {
        matches!(target, PeerEndpoint::Http(_))
    }

    fn request<'a>(
        &'a self,
        target: &'a PeerEndpoint,
        path: &'a str,
        body: Option<Value>,
    ) -> BoxFuture<'a, Result<Value>> {
        Box::pin(async move {
            let PeerEndpoint::Http(url) = target else {
                return Err("HTTP 传输不支持该地址".into());
            };
            let request = if let Some(v) = body {
                self.client.post(format!("{url}{path}")).json(&v)
            } else {
                self.client.get(format!("{url}{path}"))
            };
            let mut r = request.send().await.map_err(|e| format!("{e:?}"))?;
            if !r.status().is_success() {
                return Err(format!("设备请求失败 ({})", r.status()));
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = r.chunk().await.map_err(|e| e.to_string())? {
                if bytes.len() + chunk.len() > MAX_BODY {
                    return Err("设备响应过大".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())
        })
    }
}

/// Dispatches outgoing RPCs to the first registered transport supporting the target endpoint.
#[derive(Clone)]
pub struct TransportRouter {
    transports: Arc<Vec<Arc<dyn Transport>>>,
}

impl TransportRouter {
    pub fn new(transports: Vec<Arc<dyn Transport>>) -> Self {
        Self {
            transports: Arc::new(transports),
        }
    }

    pub async fn request(&self, url: &str, path: &str, body: Option<Value>) -> Result<Value> {
        let target = parse_endpoint(url)?;
        for transport in self.transports.iter() {
            if transport.supports(&target) {
                return transport.request(&target, path, body).await;
            }
        }
        match target {
            PeerEndpoint::Iroh { .. } => Err("请先开启跨网同步（Iroh）后再连接 iroh:// 地址".into()),
            PeerEndpoint::Http(_) => Err("未找到可用的局域网传输通道".into()),
        }
    }
}
