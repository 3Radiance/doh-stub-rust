# doh-stub

**[Read in English →](./README.md)**

[![CI](https://github.com/ImSavsis/doh-stub/actions/workflows/ci.yml/badge.svg)](https://github.com/ImSavsis/doh-stub/actions/workflows/ci.yml)
[![license](https://img.shields.io/github/license/ImSavsis/doh-stub.svg)](https://github.com/ImSavsis/doh-stub/blob/master/LICENSE)

Кроссплатформенный локальный DNS-over-HTTPS (DoH) резолвер на Rust. Преобразует обычные локальные UDP DNS-запросы в шифрованные HTTPS POST-запросы к DoH-провайдеру.

Ходит на DoH-серверы напрямую по IP, минуя системный DNS. Автоматически обнаруживает новых провайдеров и кэширует их в локальный JSON-конфиг.

```mermaid
sequenceDiagram
    participant OS as Система (UDP)
    participant Stub as doh-stub-rust
    participant JSON as bootstrap.json
    participant DoH as DoH-провайдер (HTTPS)

    OS->>Stub: Обычный DNS-запрос (UDP:53/5300)
    Note over Stub: Проверка bootstrap.json / Резолв IP / Fallback
    Stub->>DoH: HTTPS POST (прямой IP + SNI-эмуляция)
    DoH->>Stub: Ответ (application/dns-message)
    Stub->>OS: Обычный DNS-ответ (UDP)
```

## Возможности

- **Прямое подключение по IP** — Не зависит от системного DNS для резолва самого DoH-сервера.
- **Автообнаружение провайдеров** — Определяет, передан ли `--doh` как IP или домен, резолвит домен через bootstrap-провайдеров и валидирует URL.
- **Fallback между провайдерами** — Если основной провайдер падает (таймаут, ошибка соединения, HTTP-ошибка), автоматически переключается на следующего из `bootstrap.json`.
- **Bootstrap-кэш (`bootstrap.json`)** — Новые DoH-URL через `-d` автоматически резолвятся и дописываются в конфиг для мгновенного старта в следующий раз.
- **Эмуляция браузерного TLS** — Использует `wreq` с TLS-отпечатком Firefox 136 для обхода DPI и блокировок.
- **Таймауты запросов** — 10с общий таймаут, 5с на TCP+TLS, 8с на один запрос. Не виснет на мёртвых соединениях.

## Сборка

```bash
cargo build --release
```

## Использование

По умолчанию (порт 5300, Cloudflare primary, Google fallback):

```bash
./target/release/doh-stub-rust
```

Кастомный порт и провайдер:

```bash
./target/release/doh-stub-rust -p 53 -d https://dns.google/dns-query
```

Прямой IP (резолв домена не нужен):

```bash
./target/release/doh-stub-rust -p 53 -d https://1.1.1.1/dns-query
```

### Флаги CLI

| Флаг | Описание | По умолчанию |
|------|----------|--------------|
| `-p` | Локальный UDP-порт для входящих DNS-запросов | `5300` |
| `-d` | URL DoH-провайдера | `https://cloudflare-dns.com/dns-query` |

## Валидация провайдеров

При передаче нового провайдера через `-d` выполняется проверка:

- **Схема** — только `https`, `http` отклоняется.
- **Путь** — должен содержать `dns-query`, стандартный DoH-эндпоинт.
- **Хост** — автоматически определяется как IP или домен. Домены резолвятся через существующих bootstrap-провайдеров.

## Поведение fallback

Провайдеры перебираются по порядку до первого успеха:

1. Провайдер из `-d` (если есть в `bootstrap.json`, он поднимается на первое место).
2. Остальные провайдеры из `bootstrap.json` в порядке хранения.

Если провайдер падает — ошибка логируется и сразу пробуется следующий. Если все упали — запрос сбрасывается с записью в лог.

## Конфигурация (`bootstrap.json`)

Создаётся автоматически при первом запуске:

```json
{
  "primary": "cloudflare",
  "providers": [
    {
      "name": "cloudflare-dns",
      "domain": "cloudflare-dns.com",
      "url": "https://cloudflare-dns.com/dns-query",
      "ips": ["104.16.249.249", "104.16.248.249"]
    },
    {
      "name": "google-dns",
      "domain": "dns.google",
      "url": "https://dns.google/dns-query",
      "ips": ["8.8.8.8", "8.8.4.4"]
    }
  ]
}
```

Новые провайдеры, обнаруженные через `-d`, дописываются в файл автоматически.

## Системная интеграция (NixOS / systemd)

Для перехвата всего системного DNS-трафика забиндись на 53 порт и пропиши `127.0.0.1` как системный резолвер.

Пример сервиса в NixOS:

```nix
systemd.services.doh-stub = {
  description = "Custom Rust DoH Resolver";
  after = [ "network.target" ];
  wantedBy = [ "multi-user.target" ];
  serviceConfig = {
    ExecStart = "/path/to/doh-stub-rust -p 53 -d https://your-doh-provider.com/dns-query";
    WorkingDirectory = "/path/to/doh-dir";
    Restart = "always";
    RestartSec = "3s";
    User = "root";
  };
};
```

## Лицензия

MIT
