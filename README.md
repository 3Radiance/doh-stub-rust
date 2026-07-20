# doh-stub

[![CI](https://github.com/ImSavsis/doh-stub/actions/workflows/ci.yml/badge.svg)](https://github.com/ImSavsis/doh-stub/actions/workflows/ci.yml)
[![license](https://img.shields.io/github/license/ImSavsis/doh-stub.svg)](https://github.com/ImSavsis/doh-stub/blob/master/LICENSE)

локальный DNS-резолвер на rust, который сам ходит наружу по DNS-over-HTTPS. провайдер/DPI видит только HTTPS к cloudflare, а не голые DNS-запросы.

```mermaid
sequenceDiagram
    OS->>doh-stub: обычный DNS-запрос (UDP)
    doh-stub->>Cloudflare: тот же запрос, но внутри HTTPS POST
    Cloudflare->>doh-stub: ответ внутри HTTPS
    doh-stub->>OS: обычный DNS-ответ (UDP)
```

## сборка

```
cargo build --release
```

## юзать

```
doh-stub-rust\target/release/doh-stub-rust -p 5300 -d https://cloudflare-dns.com/dns-query
```

по умолчанию слушает `127.0.0.1:5300` и форвардит на `cloudflare-dns.com`. чтобы реально подменить системный DNS — нужен порт 53 и админ, плюс прописать `127.0.0.1` в настройках сети.

