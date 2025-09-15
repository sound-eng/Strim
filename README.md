# Strim

Unified audio streaming tool with server and client modes.

## Build

```bash
cargo build -p strim
```

## Usage

Subcommands (preferred):

```bash
# server mode (default port is from code DEFAULT_PORT)
cargo run -p strim -- server -p 8080

# client mode
cargo run -p strim -- client -H localhost -p 8080
```

Default server mode (no subcommand):

```bash
# defaults to 'server' when no subcommand is provided
cargo run -p strim -- -p 8080
```

Options:
- server: `-p, --port <PORT>` (defaults to `DEFAULT_PORT`), `-d, --device-id <STRING>`
- client: `-H, --host <HOST>` (default `localhost`), `-p, --port <PORT>` (defaults to `DEFAULT_PORT`)

### Help

Server help:

```bash
cargo run -p strim -- server --help

Run server mode (default)

Usage: strim server [OPTIONS]

Options:
  -p, --port <PORT>            Port to bind server to (for server mode) [default: 8080]
  -d, --device-id <DEVICE_ID>  Audio device ID (for server mode) [default: ]
  -h, --help                   Print help
```

Client help:

```bash
cargo run -p strim -- client --help

Run client mode

Usage: strim client [OPTIONS]

Options:
  -H, --host <HOST>  Host of the server to connect to (for client mode) [default: localhost]
  -p, --port <PORT>  Port on the server to connect to (for client mode) [default: 8080]
  -h, --help         Print help
```
