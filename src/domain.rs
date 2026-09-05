use rand::seq::SliceRandom;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::timeout;
use wreq::Client;
use wreq_util::Emulation;

use crate::BoostrapConfig;

pub async fn resolve_domain(
    domain: &str,
    config: &Arc<RwLock<BoostrapConfig>>,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut last_err = None;

    let mut providers = config.read().await.providers.clone();
    {
        let mut rng = rand::thread_rng();
        providers.shuffle(&mut rng);
    }

    for provider in &providers {
        let ip = {
            let mut rng = rand::thread_rng();
            match provider.ips.choose(&mut rng) {
                Some(ip) => ip.clone(),
                None => continue,
            }
        };

        let sock_addr: SocketAddr = format!("{}:443", ip).parse()?;
        let domain_static = provider.domain.clone();

        let client = match Client::builder()
            .emulation(Emulation::random())
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
            0x00, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
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
                eprintln!(
                    "[WARN] {} did not respond for resolving {}: {}",
                    provider.name, domain, e
                );
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
                bytes[p],
                bytes[p + 1],
                bytes[p + 2],
                bytes[p + 3]
            ));
        }
        p += rdlen;
    }
    None
}
