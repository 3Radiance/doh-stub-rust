use clap::Parser;
use reqwest::Client;
use std::net::SocketAddr;
use std::result;
use std::sync::Arc;
use tokio::net::UdpSocket;

#[derive(Parser, Debug)]
#[command(author, version, about = "Кроссплатформенный DoH-stub на Rust")]
struct Args {
    #[arg(short, long, default_value = "5300")]
    port: u16,
    #[arg(short, long, default_value = "https://cloudflare-dns.com/dns-query")]
    doh: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    let socket = UdpSocket::bind(&addr).await?;
    let client = Client::new();
    let socket = Arc::new(socket);
    let mut buf = [0u8; 512];
    loop {
        let (len, client_addr) = socket.recv_from(&mut buf).await?;
        let q_bytes = buf[..len].to_vec();
        let socket_clone = Arc::clone(&socket);
        let client_clone = client.clone();
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
    client: &Client,
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
