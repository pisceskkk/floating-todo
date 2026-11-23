//! Cross-network P2P + relay transport powered by Iroh (QUIC).
use crate::transport::{BoxFuture, PeerEndpoint, Result, Transport, MAX_BODY};
use iroh::{
    endpoint::{presets, Connection},
    Endpoint, EndpointAddr, EndpointId, RelayMap, RelayMode, RelayUrl, SecretKey,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Mutex, Semaphore},
    task::JoinHandle,
};

pub const IROH_ALPN: &[u8] = b"floating-todo-lan/1";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const RPC_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Clone, Debug)]
pub enum IrohRelayOption {
    /// Use n0's default public relays (Singapore, US, EU) with automatic DNS/pkarr discovery.
    DefaultPublic,
    /// Use a custom self-hosted `iroh-relay` server.
    Custom(RelayUrl),
    /// Disable relays (offline local unit tests only).
    #[cfg(test)]
    LocalOnly,
}

#[derive(Serialize, Deserialize)]
struct WireRequest {
    path: String,
    #[serde(default)]
    body: Option<Value>,
}

#[derive(Serialize, Deserialize)]
struct WireResponse {
    ok: bool,
    #[serde(default)]
    value: Option<Value>,
    #[serde(default)]
    error: Option<String>,
}

pub fn load_or_generate_secret(existing_hex: Option<&str>) -> Result<(SecretKey, String)> {
    if let Some(raw) = existing_hex.filter(|s| !s.is_empty()) {
        let bytes = hex::decode(raw).map_err(|_| "Iroh 节点密钥损坏".to_string())?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| "Iroh 节点密钥长度无效".to_string())?;
        let key = SecretKey::from_bytes(&arr);
        return Ok((key, raw.to_owned()));
    }
    let key = SecretKey::generate();
    let encoded = hex::encode(key.to_bytes());
    Ok((key, encoded))
}

pub struct IrohTransport {
    endpoint: Endpoint,
    custom_relay: Option<RelayUrl>,
    connections: Mutex<HashMap<EndpointId, Connection>>,
}

impl IrohTransport {
    pub async fn bind(secret_key: SecretKey, relay: IrohRelayOption) -> Result<Arc<Self>> {
        let (builder, custom_relay) = match relay {
            IrohRelayOption::DefaultPublic => (
                Endpoint::builder(presets::N0).relay_mode(RelayMode::Default),
                None,
            ),
            IrohRelayOption::Custom(url) => {
                let map = RelayMap::from(url.clone());
                (
                    Endpoint::builder(presets::N0).relay_mode(RelayMode::Custom(map)),
                    Some(url),
                )
            }
            #[cfg(test)]
            IrohRelayOption::LocalOnly => (
                Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled),
                None,
            ),
        };
        let endpoint = builder
            .secret_key(secret_key)
            .alpns(vec![IROH_ALPN.to_vec()])
            .bind()
            .await
            .map_err(|e| format!("启动 Iroh 跨网通道失败：{e}"))?;
        Ok(Arc::new(Self {
            endpoint,
            custom_relay,
            connections: Mutex::new(HashMap::new()),
        }))
    }

    pub fn endpoint_id(&self) -> EndpointId {
        self.endpoint.id()
    }

    pub fn custom_relay_url(&self) -> Option<&RelayUrl> {
        self.custom_relay.as_ref()
    }

    pub fn home_relay(&self) -> Option<String> {
        self.endpoint
            .addr()
            .relay_urls()
            .next()
            .map(|u| u.as_str().trim_end_matches('/').to_owned())
    }

    /// Active relay URL (prefers custom self-hosted relay, falls back to current public home relay).
    pub fn active_relay_url(&self) -> Option<String> {
        if let Some(relay) = &self.custom_relay {
            Some(relay.as_str().trim_end_matches('/').to_owned())
        } else {
            self.home_relay()
        }
    }

    /// Shareable cross-network address for other devices to discover & pair with this node.
    /// Automatically appends `?relay=...` when a relay is known so peers on different relays
    /// or restricted networks can connect directly without relying on DNS/Pkarr lookup.
    pub fn share_address(&self) -> String {
        let id = self.endpoint.id();
        if let Some(relay) = self.active_relay_url() {
            format!("iroh://{id}?relay={relay}")
        } else {
            format!("iroh://{id}")
        }
    }

    #[cfg(test)]
    pub fn local_direct_addr(&self) -> Option<std::net::SocketAddr> {
        let addr = self.endpoint.addr();
        let found = addr
            .ip_addrs()
            .find(|a| a.is_ipv4() && a.ip().is_loopback())
            .copied()
            .or_else(|| {
                addr.ip_addrs().find(|a| a.is_ipv4()).map(|a| {
                    std::net::SocketAddr::new(std::net::Ipv4Addr::LOCALHOST.into(), a.port())
                })
            });
        found
    }

    #[cfg(test)]
    pub fn test_direct_address(&self) -> String {
        let id = self.endpoint.id();
        if let Some(sock) = self.local_direct_addr() {
            format!("iroh://{id}?addr={sock}")
        } else {
            format!("iroh://{id}")
        }
    }

    pub fn close_in_background(&self) {
        let ep = self.endpoint.clone();
        tokio::spawn(async move {
            ep.close().await;
        });
    }

    pub fn spawn_server(
        self: &Arc<Self>,
        permits: Arc<Semaphore>,
        handler: Arc<dyn Fn(EndpointId, String, Option<Value>) -> Result<Value> + Send + Sync>,
    ) -> JoinHandle<()> {
        let ep = self.endpoint.clone();
        tokio::spawn(async move {
            while let Some(incoming) = ep.accept().await {
                let permits = permits.clone();
                let handler = handler.clone();
                tokio::spawn(async move {
                    let Ok(conn) = incoming.await else {
                        return;
                    };
                    let remote_id = conn.remote_id();
                    loop {
                        let Ok((send, recv)) = conn.accept_bi().await else {
                            break;
                        };
                        let permits = permits.clone();
                        let handler = handler.clone();
                        tokio::spawn(async move {
                            handle_stream(remote_id, send, recv, permits, handler).await;
                        });
                    }
                });
            }
        })
    }

    async fn get_or_connect(
        &self,
        id: EndpointId,
        relay: Option<&RelayUrl>,
        #[cfg(test)] direct: Option<std::net::SocketAddr>,
    ) -> Result<Connection> {
        {
            let mut guard = self.connections.lock().await;
            if let Some(conn) = guard.get(&id) {
                if conn.close_reason().is_none() {
                    return Ok(conn.clone());
                }
                guard.remove(&id);
            }
        }
        let mut addr = EndpointAddr::new(id);
        if let Some(r) = relay.or(self.custom_relay.as_ref()) {
            addr = addr.with_relay_url(r.clone());
        }
        #[cfg(test)]
        if let Some(sock) = direct {
            addr = addr.with_ip_addr(sock);
        }
        let conn = tokio::time::timeout(CONNECT_TIMEOUT, self.endpoint.connect(addr, IROH_ALPN))
            .await
            .map_err(|_| "连接 Iroh 节点超时".to_string())?
            .map_err(|e| format!("连接 Iroh 节点失败：{e}"))?;
        self.connections.lock().await.insert(id, conn.clone());
        Ok(conn)
    }

    async fn send_on_conn(
        conn: &Connection,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let (mut send, mut recv) = conn
            .open_bi()
            .await
            .map_err(|e| format!("打开 Iroh 通道流失败：{e}"))?;
        let req = WireRequest {
            path: path.to_owned(),
            body,
        };
        let bytes = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_BODY {
            return Err("请求内容过大".into());
        }
        write_frame(&mut send, &bytes).await?;
        send.finish().map_err(|e| format!("发送结束失败：{e}"))?;

        let resp_bytes = read_frame(&mut recv).await?;
        let resp: WireResponse = serde_json::from_slice(&resp_bytes).map_err(|e| e.to_string())?;
        if resp.ok {
            resp.value.ok_or_else(|| "设备响应为空".into())
        } else {
            Err(resp.error.unwrap_or_else(|| "设备请求失败".into()))
        }
    }
}

impl Transport for IrohTransport {
    fn supports(&self, target: &PeerEndpoint) -> bool {
        matches!(target, PeerEndpoint::Iroh { .. })
    }

    fn request<'a>(
        &'a self,
        target: &'a PeerEndpoint,
        path: &'a str,
        body: Option<Value>,
    ) -> BoxFuture<'a, Result<Value>> {
        Box::pin(async move {
            let PeerEndpoint::Iroh {
                id,
                relay,
                #[cfg(test)]
                direct,
            } = target
            else {
                return Err("Iroh 传输不支持该地址".into());
            };
            let conn = self
                .get_or_connect(
                    *id,
                    relay.as_ref(),
                    #[cfg(test)]
                    *direct,
                )
                .await?;
            let first = tokio::time::timeout(
                RPC_TIMEOUT,
                Self::send_on_conn(&conn, path, body.clone()),
            )
            .await;
            match first {
                Ok(Ok(val)) => Ok(val),
                Ok(Err(_)) | Err(_) if conn.close_reason().is_some() => {
                    // Stale pooled connection: evict and retry once with a fresh handshake.
                    self.connections.lock().await.remove(id);
                    let fresh = self
                        .get_or_connect(
                            *id,
                            relay.as_ref(),
                            #[cfg(test)]
                            *direct,
                        )
                        .await?;
                    tokio::time::timeout(RPC_TIMEOUT, Self::send_on_conn(&fresh, path, body))
                        .await
                        .map_err(|_| "Iroh 请求超时".to_string())?
                }
                Ok(Err(e)) => Err(e),
                Err(_) => Err("Iroh 请求超时".into()),
            }
        })
    }
}

async fn handle_stream(
    remote_id: EndpointId,
    mut send: iroh::endpoint::SendStream,
    mut recv: iroh::endpoint::RecvStream,
    permits: Arc<Semaphore>,
    handler: Arc<dyn Fn(EndpointId, String, Option<Value>) -> Result<Value> + Send + Sync>,
) {
    let Ok(_permit) = permits.try_acquire_owned() else {
        let _ = write_response(
            &mut send,
            WireResponse {
                ok: false,
                value: None,
                error: Some("设备请求繁忙，请稍后重试".into()),
            },
        )
        .await;
        return;
    };
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let bytes = read_frame(&mut recv).await?;
        let req: WireRequest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        handler(remote_id, req.path, req.body)
    })
    .await;
    let resp = match result {
        Ok(Ok(value)) => WireResponse {
            ok: true,
            value: Some(value),
            error: None,
        },
        Ok(Err(err)) => WireResponse {
            ok: false,
            value: None,
            error: Some(err),
        },
        Err(_) => WireResponse {
            ok: false,
            value: None,
            error: Some("设备处理超时".into()),
        },
    };
    let _ = write_response(&mut send, resp).await;
}

async fn write_response(
    send: &mut iroh::endpoint::SendStream,
    resp: WireResponse,
) -> Result<()> {
    let bytes = serde_json::to_vec(&resp).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BODY {
        let fallback = serde_json::to_vec(&WireResponse {
            ok: false,
            value: None,
            error: Some("响应内容过大".into()),
        })
        .unwrap_or_default();
        write_frame(send, &fallback).await?;
    } else {
        write_frame(send, &bytes).await?;
    }
    let _ = send.finish();
    Ok(())
}

async fn write_frame<W: AsyncWriteExt + Unpin>(writer: &mut W, payload: &[u8]) -> Result<()> {
    let len = u32::try_from(payload.len()).map_err(|_| "消息过大".to_string())?;
    writer
        .write_all(&len.to_be_bytes())
        .await
        .map_err(|e| format!("写入消息头失败：{e}"))?;
    writer
        .write_all(payload)
        .await
        .map_err(|e| format!("写入消息体失败：{e}"))?;
    Ok(())
}

async fn read_frame<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    reader
        .read_exact(&mut len_buf)
        .await
        .map_err(|e| format!("读取消息头失败：{e}"))?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > MAX_BODY {
        return Err("设备响应大小无效".into());
    }
    let mut buf = vec![0u8; len];
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|e| format!("读取消息体失败：{e}"))?;
    Ok(buf)
}
