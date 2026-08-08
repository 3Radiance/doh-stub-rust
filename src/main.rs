use clap::Parser;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use wreq::Client;
use wreq_util::Emulation;
use serde::{Deserialize , Serialize};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use rand::seq::SliceRandom;

#[derive(Parser, Debug)]
#[command(author, version, about = "Кроссплатформенный DoH-stub на Rust")]
struct Args {
    #[arg(short, long, default_value = "5300")]
    port: u16,
    #[arg(short, long, default_value = "https://cloudflare-dns.com/dns-query")]
    doh: String,
}
#[derive(Debug, Deserialize ,Serialize, Clone)]
struct Provider {
    name: String,
    domain: String,
    url: String,
    ips: Vec<String>
}
#[derive(Debug, Deserialize ,Serialize, Clone)]
struct BoostrapConfig {
    primary: String,
    providers: Vec<Provider>
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    let socket = UdpSocket::bind(&addr).await?;
    let mut config = load_json("bootstrap.json")?;
 
    let mut found: Option<&Provider> = config.providers.iter().find(|p| p.url == args.doh);
    
    let client = if let Some(provider) = found {
        println!("Найден провайдер из JSON: {}", provider.name);
        println!("Домен для SNI: {}", provider.domain);
        let mut rng = rand::thread_rng();
            
        let ip_str = if let Some(ips_rand) = provider.ips.choose(&mut rng) {
            ips_rand
        } else {
            return  Err("Масив пуст".into());
        };
        let sock_addr: SocketAddr = format!("{}:443", ip_str).parse()?;
        println!("Запросы к {} пойдут напрямую на IP: {}", provider.domain, sock_addr);
        let domain_static: &'static str = Box::leak(provider.domain.clone().into_boxed_str());

        Client::builder()
        .emulation(Emulation::Firefox136)
        .resolve(domain_static, sock_addr)
        .build()?
    } else {
        let parsed_url = url::Url::parse(&args.doh)?;
        let host_str = parsed_url.host_str().ok_or("Не удалось извлечь хост из URL")?;

        let ip_str = if host_str.parse::<std::net::IpAddr>().is_ok() {
            host_str.to_string()
        } else {
            println!("Домен {} не найден в JSON. Запрашиваем IP...", host_str);
            resolve_domain(host_str, &config).await?
        };

        println!("IP: {}", ip_str);
        
        let new_provider = Provider {
            name: host_str.to_string(),
            domain: host_str.to_string(),
            url: args.doh.clone(),
            ips: vec![ip_str.clone()],
        };
        config.providers.push(new_provider);
        if let Err(e) = save_json("bootstrap.json", &config) {
            eprintln!("Не удалось сохранить новый провайдер в JSON: {}", e);
        }

        let sock_addr: SocketAddr = format!("{}:443", ip_str).parse()?;
        let domain_static: &'static str = Box::leak(host_str.to_string().into_boxed_str());

        Client::builder()
            .emulation(Emulation::Firefox136)
            .resolve(domain_static, sock_addr)
            .build()?
    };
    
    let client = Arc::new(client);
    
    let socket = Arc::new(socket);

    let mut buf = [0u8; 512];
    loop {
        let (len, client_addr) = socket.recv_from(&mut buf).await?;
        let q_bytes = buf[..len].to_vec();
        let socket_clone = Arc::clone(&socket);
        let client_clone = Arc::clone(&client);
        let doh_url_clone = args.doh.clone();

        tokio::spawn(async move {
            match doh_forward(&client_clone, &doh_url_clone, q_bytes).await {
                Ok(answer_bytes) => {
                    if let Err(e) = socket_clone.send_to(&answer_bytes, client_addr).await {
                        eprintln!("Ошибка отправки UDP ответа клиенту {}: {}", client_addr, e);
                    } else {
                        println!(
                            "Резолв отправлен на {}, байт: {}",
                            client_addr,
                            answer_bytes.len()
                        );
                    }
                }
                Err(e) => {
                    eprintln!("Ошибка DoH запроса для {}: {}", client_addr, e);
                }
            }
        });
    }
    Ok(())
}
async fn doh_forward(
    client: &wreq::Client,
    url: &str,
    q: Vec<u8>,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let res = client
        .post(url)
        .header("Content-Type", "application/dns-message")
        .header("Accept", "application/dns-message")
        .body(q)
        .send()
        .await?;
    if !res.status().is_success() {
        return Err(format!("Status: {}", res.status()).into());
    }
    let bytes = res.bytes().await?;

    Ok(bytes.to_vec())
}
fn load_json<P: AsRef<Path>>(path: P) -> Result<BoostrapConfig, Box<dyn std::error::Error>> {
    let path = path.as_ref();
    if !path.exists() {
        println!("Файл не найден создание базового файлика {}",path.display());

        let def_conf: BoostrapConfig = BoostrapConfig {
            primary: "dns".to_string(),
            providers: vec![
                Provider {
                    name: "cloudflare-dns".to_string(),
                    domain: "cloudflare-dns.com".to_string(),
                    url: "https://cloudflare-dns.com/dns-query".to_string(),
                    ips: vec!["104.16.249.249".to_string(),"104.16.248.249".to_string()],
                },
                Provider {
                    name: "google-dns".to_string(),
                    domain: "dns.google".to_string(),
                    url: "https://dns.google/dns-query".to_string(),
                    ips: vec!["8.8.8.8".to_string(),"8.8.4.4".to_string()],
                },
            ],
        };
        let file = File::create(path)?;
        serde_json::to_writer_pretty(file, &def_conf)?;
        return Ok(def_conf);

    }
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let config = serde_json::from_reader(reader)?;

    Ok(config)
}
fn save_json<P: AsRef<Path>>(path: P,config: &BoostrapConfig) -> Result<(), Box<dyn std::error::Error>> {
    let file = File::create(path)?;
    serde_json::to_writer_pretty(file, config)?;
    Ok(())
}
async fn resolve_domain(
    domain: &str,
    config: &BoostrapConfig
) -> Result<String,Box<dyn std::error::Error>> {
     let mut rng = rand::thread_rng();
     
     let provider = config.providers
     .choose(&mut rng)
     .ok_or("Список провайдеров пуст")?;
     
     let ip = provider.ips
     .choose(&mut rng)
     .ok_or(format!("Список Айпи у провайдера {} пуст",provider.name))?;

    let sock_addr: SocketAddr = format!("{}:443", ip).parse()?;

    let mut packet = Vec::new();
    packet.extend_from_slice(&[
     0x00, 0x01, 
     0x01, 0x00, 
     0x00, 0x01, 
     0x00, 0x00, 
     0x00, 0x00, 
     0x00, 0x00, 
    ]);
    
    for part in domain.split(".") {
        packet.push(part.len() as u8);
        packet.extend_from_slice(part.as_bytes());
    }

    packet.push(0x00);

    packet.extend_from_slice(&[
      0x00, 0x01, 
      0x00, 0x01, 
    ]);
    let domain_static: &'static str = Box::leak(provider.domain.clone().into_boxed_str());
    let client = 
    Client::builder()
        .emulation(Emulation::Firefox136)
        .resolve(domain_static, sock_addr)
        .build()?;
    
    let res = client
        .post(&provider.url)
        .header("Content-Type", "application/dns-message")
        .header("Accept", "application/dns-message")
        .body(packet)
        .send()
        .await?;

    if !res.status().is_success() {
        return Err(format!("Status: {}", res.status()).into());
    }
    let bytes = res.bytes().await?;

    let mut p = 12;
     while p < bytes.len() && bytes[p] != 0 {
        let len = bytes[p] as usize;
        p += 1 + len;
     }
     p += 5;

     if p < bytes.len() && (bytes[p] & 0xC0) == 0xC0 {
      p += 2;
    } else { 
   
    while p < bytes.len() && bytes[p] != 0 {
        p += 1 + bytes[p] as usize;
     }
     p += 1;
    }

    p += 8;


    if p + 2 + 4 <= bytes.len() {
    let rd_len = u16::from_be_bytes([bytes[p], bytes[p + 1]]);
    p += 2;

    if rd_len == 4 {
       
        let b1 = bytes[p];
        let b2 = bytes[p + 1];
        let b3 = bytes[p + 2];
        let b4 = bytes[p + 3];

        let resolved_ip = format!("{}.{}.{}.{}", b1, b2, b3, b4);
        return Ok(resolved_ip);
     }
    }

   Err("Не удалось распарсить A-запись из DNS ответа".into())
}