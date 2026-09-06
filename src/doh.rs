use rand::seq::SliceRandom;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::timeout;
use wreq::Client;
use wreq_util::Emulation;

use crate::Args;
use crate::BoostrapConfig;
use crate::DoHClient;
use crate::Provider;
use crate::domain;
use crate::padding::{PaddingMethod, PaddingPlacement, build_padding_header};

pub fn build_client(provider: &Provider) -> Result<Client, Box<dyn std::error::Error>> {
    let mut rng = rand::thread_rng();
    let ip_str = provider
        .ips
        .choose(&mut rng)
        .ok_or_else(|| format!("IP array empty for {}", provider.name))?;

    let sock_addr: SocketAddr = format!("{}:443", ip_str).parse()?;
    let domain = provider.domain.clone();

    Ok(Client::builder()
        .emulation(Emulation::random())
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .resolve(domain, sock_addr)
        .build()?)
}

pub async fn detect_doh_provider(
    args: &Args,
    config: &Arc<RwLock<BoostrapConfig>>,
) -> Result<Provider, Box<dyn std::error::Error>> {
    let parsed = url::Url::parse(&args.doh)?;
    let scheme = parsed.scheme();
    if scheme != "https" {
        return Err(format!("Invalid protocol: {}. Only HTTPS allowed", scheme).into());
    }

    let path = parsed.path();
    if !path.contains("dns-query") {
        return Err(format!("Invalid path: {}. Expected /dns-query", path).into());
    }

    let host_str = parsed.host_str().ok_or("Failed to extract host from URL")?;

    let (name, domain, ips) = if let Ok(ip) = host_str.parse::<std::net::IpAddr>() {
        let ip_str = ip.to_string();
        (ip_str.clone(), ip_str.clone(), vec![ip_str])
    } else {
        println!("[INIT] Resolving domain {}...", host_str);
        let ip_str = domain::resolve_domain(host_str, config, args).await?;
        (host_str.to_string(), host_str.to_string(), vec![ip_str])
    };

    Ok(Provider {
        name,
        domain,
        url: args.doh.to_string(),
        ips,
    })
}

pub async fn doh_forward_with_fallback(
    clients: &Arc<RwLock<Vec<DoHClient>>>,
    q: Vec<u8>,
    args: &Args,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut last_err = None;

    for (idx, doh) in clients.read().await.iter().enumerate() {
        match doh_forward(&doh.client, &doh.url, q.clone(), args).await {
            Ok(bytes) => {
                if idx > 0 {
                    println!("[FALLBACK] Fallback provider triggered: {}", doh.name);
                }
                return Ok(bytes);
            }
            Err(e) => {
                eprintln!("[ERR] {} unavailable: {}", doh.name, e);
                last_err = Some(e);
            }
        }
    }

    Err(format!("All providers down. Last error: {:?}", last_err).into())
}

pub async fn doh_forward(
    client: &wreq::Client,
    url: &str,
    q: Vec<u8>,
    args: &Args,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let res = timeout(Duration::from_secs(8), async {
        let mut req = client
            .post(url)
            .header("Content-Type", "application/dns-message")
            .header("Accept", "application/dns-message");

        let pad_size = args.padding.sample();
        if pad_size > 0 {
            if let Some((name, value)) = build_padding_header(
                PaddingMethod::Tokenish,
                PaddingPlacement::Header {
                    header_name: "X-Padding".to_string(),
                },
                pad_size,
            ) {
                req = req.header(name, value);
            }
        }

        req.body(q).send().await
    })
    .await
    .map_err(|_| "Request timeout (8 sec)".to_string())??;

    if !res.status().is_success() {
        return Err(format!("HTTP Status: {}", res.status()).into());
    }

    let bytes = res.bytes().await?;
    Ok(bytes.to_vec())
}
