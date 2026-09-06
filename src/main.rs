use clap::Parser;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::fs::File;
use std::io::BufReader;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::RwLock;
use wreq::Client;

mod doh;
mod domain;
mod padding;

#[derive(Debug, Clone)]
pub struct PaddingRange {
    pub from: u16,
    pub to: u16,
}

impl PaddingRange {
    pub fn sample(&self) -> usize {
        if self.to == 0 {
            return 0;
        }
        if self.from == self.to {
            return self.from as usize;
        }
        use rand::Rng;
        rand::thread_rng().gen_range(self.from..=self.to) as usize
    }

    pub fn is_disabled(&self) -> bool {
        self.to == 0
    }
}

impl std::str::FromStr for PaddingRange {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((a, b)) = s.split_once('-') {
            let from = a
                .parse::<u16>()
                .map_err(|_| format!("invalid range start: {}", a))?;
            let to = b
                .parse::<u16>()
                .map_err(|_| format!("invalid range end: {}", b))?;
            if from > to {
                return Err(format!("range start {} > end {}", from, to));
            }
            Ok(PaddingRange { from, to })
        } else {
            let n = s
                .parse::<u16>()
                .map_err(|_| format!("invalid padding value: {}", s))?;
            Ok(PaddingRange { from: n, to: n })
        }
    }
}

#[derive(Parser, Debug, Clone)]
#[command(
    author,
    version,
    about = "Cross-platform DoH stub in Rust with fallback"
)]
pub struct Args {
    #[arg(short, long, default_value = "5300")]
    port: u16,
    #[arg(short, long, default_value = "https://cloudflare-dns.com/dns-query")]
    doh: String,
    #[arg(short = 'P', long, default_value = "0")]
    padding: PaddingRange,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Provider {
    name: String,
    domain: String,
    url: String,
    ips: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct BoostrapConfig {
    primary: String,
    providers: Vec<Provider>,
}

#[derive(Clone)]
pub struct DoHClient {
    client: Arc<Client>,
    url: String,
    name: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    let socket = UdpSocket::bind(&addr).await?;
    let config = Arc::new(RwLock::new(load_json("bootstrap.json")?));

    let doh_clients: Arc<RwLock<Vec<DoHClient>>> = Arc::new(RwLock::new(Vec::new()));

    for provider in &config.read().await.providers {
        match doh::build_client(provider) {
            Ok(client) => {
                println!("[INIT] {} -> {:?}", provider.name, provider.ips);
                doh_clients.write().await.push(DoHClient {
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

    found_in_config(&mut config.clone(), &args, &mut doh_clients.clone()).await?;

    println!("[INFO] UDP server listening on {}", addr);
    let socket = Arc::new(socket);
    let mut buf = [0u8; 2048];

    let cfg_clone = Arc::clone(&config);
    let clients_clone = Arc::clone(&doh_clients);
    let doh_url = args.doh.clone();
    let args_clone = args.clone();
    let args_for_loop = args.clone();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        interval.tick().await;

        loop {
            interval.tick().await;
            let domain_to_resolve = {
                let config_read = cfg_clone.read().await;
                config_read
                    .providers
                    .iter()
                    .find(|p| p.url == doh_url)
                    .map(|p| p.domain.clone())
            };
            let domain = match domain_to_resolve {
                Some(d) => d,
                None => continue,
            };
            let new_ip = match domain::resolve_domain(&domain, &cfg_clone, &args_clone).await {
                Ok(ip) => ip,
                Err(e) => {
                    eprintln!("[BACKGROUND] Failed to resolve {}: {}", domain, e);
                    continue;
                }
            };

            let mut needs_update = false;
            let mut provider_to_rebuild = None;
            {
                let config_read = cfg_clone.read().await;
                if let Some(provider) = config_read.providers.iter().find(|p| p.url == doh_url) {
                    if provider.ips.is_empty() || provider.ips[0] != new_ip {
                        println!("[BACKGROUND] IP changed for {}! New IP: {}", domain, new_ip);
                        needs_update = true;

                        let mut updated_provider = provider.clone();
                        updated_provider.ips = vec![new_ip.clone()];
                        provider_to_rebuild = Some(updated_provider);
                    }
                }
            }
            if needs_update {
                if let Some(new_provider) = provider_to_rebuild {
                    let new_client_opt = {
                        match doh::build_client(&new_provider) {
                            Ok(c) => Some(c),
                            Err(e) => {
                                eprintln!(
                                    "[BACKGROUND] Failed to build new client for {}: {}",
                                    domain, e
                                );
                                None
                            }
                        }
                    };

                    if let Some(new_client) = new_client_opt {
                        let mut clients_write = clients_clone.write().await;
                        if let Some(client_ref) =
                            clients_write.iter_mut().find(|c| c.url == doh_url)
                        {
                            client_ref.client = Arc::new(new_client);
                            println!("[BACKGROUND] Replaced HTTP client for {}", domain);
                        }

                        let mut config_write = cfg_clone.write().await;
                        if let Some(provider_ref) =
                            config_write.providers.iter_mut().find(|p| p.url == doh_url)
                        {
                            provider_ref.ips = vec![new_ip.clone()];
                        }

                        drop(config_write);
                        if let Err(e) = save_json("bootstrap.json", &cfg_clone).await {
                            eprintln!("[BACKGROUND] Error saving config: {}", e);
                        } else {
                            println!("[BACKGROUND] bootstrap.json updated successfully.");
                        }
                    }
                }
            }
        }
    });

    loop {
        let (len, client_addr) = socket.recv_from(&mut buf).await?;
        println!("[DEBUG] Received {} bytes from {}", len, client_addr);
        let q_bytes = buf[..len].to_vec();

        let socket_clone = Arc::clone(&socket);
        let clients_clone = Arc::clone(&doh_clients);
        let args_req = args_for_loop.clone();

        tokio::spawn(async move {
            match doh::doh_forward_with_fallback(&clients_clone, q_bytes, &args_req).await {
                Ok(answer_bytes) => {
                    if let Err(e) = socket_clone.send_to(&answer_bytes, client_addr).await {
                        eprintln!("[UDP] Error sending response to {}: {}", client_addr, e);
                    } else {
                        println!(
                            "[OK] Resolve sent to {}, bytes: {}",
                            client_addr,
                            answer_bytes.len()
                        );
                    }
                }
                Err(e) => {
                    eprintln!("[FATAL] All providers failed for {}: {}", client_addr, e);
                }
            }
        });
    }
}

async fn found_in_config(
    config: &mut Arc<RwLock<BoostrapConfig>>,
    args: &Args,
    doh_clients: &mut Arc<RwLock<Vec<DoHClient>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let found_in_config = config
        .read()
        .await
        .providers
        .iter()
        .any(|p| p.url == args.doh);
    if !found_in_config {
        let provider = doh::detect_doh_provider(args, config).await?;
        println!(
            "[INIT] New provider: {} -> {:?}",
            provider.name, provider.ips
        );

        if let Ok(client) = doh::build_client(&provider) {
            doh_clients.write().await.push(DoHClient {
                client: Arc::new(client),
                url: provider.url.clone(),
                name: provider.name.clone(),
            });
        }
        config.write().await.providers.push(provider);
        if let Err(e) = save_json("bootstrap.json", config).await {
            eprintln!("[WARN] Failed to save bootstrap.json: {}", e);
        }
    } else {
        let pos = {
            doh_clients
                .read()
                .await
                .iter()
                .position(|c| c.url == args.doh)
        };

        if let Some(pos) = pos {
            doh_clients.write().await.swap(0, pos);
        }
    }

    if doh_clients.read().await.is_empty() {
        return Err("No DoH provider available. Check bootstrap.json.".into());
    }

    Ok(())
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

async fn save_json<P: AsRef<Path>>(
    path: P,
    config: &Arc<RwLock<BoostrapConfig>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = File::create(path)?;
    serde_json::to_writer_pretty(file, &config.read().await.clone())?;
    Ok(())
}
