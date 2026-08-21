use clap::Parser;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::timeout;
use wreq::Client;
use wreq_util::Emulation;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use rand::seq::SliceRandom;

#[derive(Parser, Debug)]
#[command(author, version, about = "Cross-platform DoH stub in Rust with fallback")]
struct Args {
    #[arg(short, long, default_value = "5300")]
    port: u16,
    #[arg(short, long, default_value = "https://cloudflare-dns.com/dns-query")]
    doh: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Provider {
    name: String,
    domain: String,
    url: String,
    ips: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct BoostrapConfig {
    primary: String,
    providers: Vec<Provider>,
}

#[derive(Clone)]
struct DoHClient {
    client: Arc<Client>,
    url: String,
    name: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    let socket = UdpSocket::bind(&addr).await?;
    let mut config = load_json("bootstrap.json")?;

    let mut doh_clients: Vec<DoHClient> = Vec::new();

    for provider in &config.providers {
        match build_client(provider) {
            Ok(client) => {
                println!("[INIT] {} -> {:?}", provider.name, provider.ips);
                doh_clients.push(DoHClient {
                    client: Arc::new(client),
                    url: provider.url.clone(),
                    name: provider.name.clone(),
                });
            }
            Err(e) => {
                eprintln!("[WARN] Skipping {}: {}", provider.name, e);
            }
        }
    }

    let found_in_config = config.providers.iter().any(|p| p.url == args.doh);
    if !found_in_config {
        let provider = detect_doh_provider(&args.doh, &config).await?;
        println!("[INIT] New provider: {} -> {:?}", provider.name, provider.ips);

        if let Ok(client) = build_client(&provider) {
            doh_clients.push(DoHClient {
                client: Arc::new(client),
                url: provider.url.clone(),
                name: provider.name.clone(),
            });
        }
        config.providers.push(provider);
        if let Err(e) = save_json("bootstrap.json", &config) {
            eprintln!("[WARN] Failed to save bootstrap.json: {}", e);
        }
    } else {
        if let Some(pos) = doh_clients.iter().position(|c| c.url == args.doh) {
            doh_clients.swap(0, pos);
        }
    }

    if doh_clients.is_empty() {
        return Err("No DoH provider available. Check bootstrap.json.".into());
    }

    println!("[INIT] Running on {}. Providers (fallback order):", addr);
    for (i, c) in doh_clients.iter().enumerate() {
        println!("  {}. {}", i + 1, c.name);
    }

    let doh_clients = Arc::new(doh_clients);
    let socket = Arc::new(socket);
    let mut buf = [0u8; 512];

    loop {
        let (len, client_addr) = socket.recv_from(&mut buf).await?;
        let q_bytes = buf[..len].to_vec();

        let socket_clone = Arc::clone(&socket);
        let clients_clone = Arc::clone(&doh_clients);

        tokio::spawn(async move {
            match doh_forward_with_fallback(&clients_clone, q_bytes).await {
                Ok(answer_bytes) => {
                    if let Err(e) = socket_clone.send_to(&answer_bytes, client_addr).await {
                        eprintln!("[UDP] Error sending response to {}: {}", client_addr, e);
                    } else {
                        println!("[OK] Resolve sent to {}, bytes: {}", client_addr, answer_bytes.len());
                    }
                }
                Err(e) => {
                    eprintln!("[FATAL] All providers failed for {}: {}", client_addr, e);
                }
            }
        });
    }
}

fn build_client(provider: &Provider) -> Result<Client, Box<dyn std::error::Error>> {
    let mut rng = rand::thread_rng();
    let ip_str = provider
        .ips
        .choose(&mut rng)
        .ok_or_else(|| format!("IP array empty for {}", provider.name))?;

    let sock_addr: SocketAddr = format!("{}:443", ip_str).parse()?;
    let domain_static: &'static str = Box::leak(provider.domain.clone().into_boxed_str());

    Ok(Client::builder()
        .emulation(Emulation::Firefox136)
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .resolve(domain_static, sock_addr)
        .build()?)
}

async fn detect_doh_provider(
    doh_url: &str,
    config: &BoostrapConfig,
) -> Result<Provider, Box<dyn std::error::Error>> {
    let parsed = url::Url::parse(doh_url)?;
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
        let ip_str = resolve_domain(host_str, config).await?;
        (host_str.to_string(), host_str.to_string(), vec![ip_str])
    };

    Ok(Provider {
        name,
        domain,
        url: doh_url.to_string(),
        ips,
    })
}

async fn doh_forward_with_fallback(
    clients: &[DoHClient],
    q: Vec<u8>,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut last_err = None;

    for (idx, doh) in clients.iter().enumerate() {
        match doh_forward(&doh.client, &doh.url, q.clone()).await {
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

    Err(format!(
        "All providers down. Last error: {:?}",
        last_err
    )
    .into())
}

async fn doh_forward(
    client: &wreq::Client,
    url: &str,
    q: Vec<u8>,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let res = timeout(Duration::from_secs(8), async {
        client
            .post(url)
            .header("Content-Type", "application/dns-message")
            .header("Accept", "application/dns-message")
            .body(q)
            .send()
            .await
    })
    .await
    .map_err(|_| "Request timeout (8 sec)".to_string())??;

    if !res.status().is_success() {
        return Err(format!("HTTP Status: {}", res.status()).into());
    }

    let bytes = res.bytes().await?;
    Ok(bytes.to_vec())
}

fn load_json<P: AsRef<Path>>(path: P) -> Result<BoostrapConfig, Box<dyn std::error::Error>> {
    let path = path.as_ref();
    if !path.exists() {
        println!("[INIT] bootstrap.json not found — creating default");
        let def_conf = BoostrapConfig {
            primary: "cloudflare".to_string(),
            providers: vec![
                Provider {
                    name: "cloudflare-dns".to_string(),
                    domain: "cloudflare-dns.com".to_string(),
                    url: "https://cloudflare-dns.com/dns-query".to_string(),
                    ips: vec!["104.16.249.249".to_string(), "104.16.248.249".to_string()],
                },
                Provider {
                    name: "google-dns".to_string(),
                    domain: "dns.google".to_string(),
                    url: "https://dns.google/dns-query".to_string(),
                    ips: vec!["8.8.8.8".to_string(), "8.8.4.4".to_string()],
                },
            ],
        };
        let file = File::create(path)?;
        serde_json::to_writer_pretty(file, &def_conf)?;
        return Ok(def_conf);
    }
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    Ok(serde_json::from_reader(reader)?)
}

fn save_json<P: AsRef<Path>>(
    path: P,
    config: &BoostrapConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = File::create(path)?;
    serde_json::to_writer_pretty(file, config)?;
    Ok(())
}

async fn resolve_domain(
    domain: &str,
    config: &BoostrapConfig,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut last_err = None;

    let mut providers = config.providers.clone();
    let mut rng = rand::thread_rng();
    providers.shuffle(&mut rng);

    for provider in &providers {
        let ip = match provider.ips.choose(&mut rng) {
            Some(ip) => ip,
            None => continue,
        };

        let sock_addr: SocketAddr = format!("{}:443", ip).parse()?;
        let domain_static: &'static str = Box::leak(provider.domain.clone().into_boxed_str());

        let client = match Client::builder()
            .emulation(Emulation::Firefox136)
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .resolve(domain_static, sock_addr)
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[WARN] Failed to build client for {}: {}", provider.name, e);
                continue;
            }
        };

        let mut packet = Vec::new();
        packet.extend_from_slice(&[
            0x00, 0x01, 
            0x01, 0x00, 
            0x00, 0x01, 
            0x00, 0x00,
            0x00, 0x00,
            0x00, 0x00, 
        ]);
        for part in domain.split('.') {
            packet.push(part.len() as u8);
            packet.extend_from_slice(part.as_bytes());
        }
        packet.push(0x00);
        packet.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);

        let res = match timeout(Duration::from_secs(8), async {
            client
                .post(&provider.url)
                .header("Content-Type", "application/dns-message")
                .header("Accept", "application/dns-message")
                .body(packet)
                .send()
                .await
        })
        .await
        {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                eprintln!("[WARN] {} did not respond for resolving {}: {}", provider.name, domain, e);
                last_err = Some(e.to_string());
                continue;
            }
            Err(_) => {
                eprintln!("[WARN] Resolve timeout via {}", provider.name);
                last_err = Some("timeout".to_string());
                continue;
            }
        };

        if !res.status().is_success() {
            last_err = Some(format!("status {}", res.status()));
            continue;
        }

        let bytes = match res.bytes().await {
            Ok(b) => b,
            Err(e) => {
                last_err = Some(e.to_string());
                continue;
            }
        };

        if let Some(ip) = parse_a_record(&bytes) {
            return Ok(ip);
        }
    }

    Err(format!(
        "Failed to resolve {} via any provider. Last error: {:?}",
        domain, last_err
    )
    .into())
}

fn parse_a_record(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 12 {
        return None;
    }
    let ancount = u16::from_be_bytes([bytes[6], bytes[7]]);
    if ancount == 0 {
        return None;
    }

    let mut p = 12;
    while p < bytes.len() && bytes[p] != 0 {
        let len = bytes[p] as usize;
        p += 1 + len;
    }
    p += 5; 

    for _ in 0..ancount {
        if p >= bytes.len() {
            break;
        }
        if (bytes[p] & 0xC0) == 0xC0 {
            p += 2;
        } else {
            while p < bytes.len() && bytes[p] != 0 {
                p += 1 + bytes[p] as usize;
            }
            p += 1;
        }
        if p + 10 > bytes.len() {
            break;
        }
        let rtype = u16::from_be_bytes([bytes[p], bytes[p + 1]]);
        let rdlen = u16::from_be_bytes([bytes[p + 8], bytes[p + 9]]) as usize;
        p += 10;

        if rtype == 1 && rdlen == 4 && p + 4 <= bytes.len() {
            return Some(format!(
                "{}.{}.{}.{}",
                bytes[p], bytes[p + 1], bytes[p + 2], bytes[p + 3]
            ));
        }
        p += rdlen;
    }
    None
}