# Strim

Unified audio streaming tool with server and client modes.

## Build

```bash
cargo build -p strim
```

## Usage

Subcommands:

```bash
# server mode (default port 8080)
cargo run -p strim -- server -p 8080

# client mode
cargo run -p strim -- client -H localhost -p 8080
```

Shortcuts:

```bash
# -s inserts the server subcommand
cargo run -p strim -- -s -p 8080

# -c inserts the client subcommand
cargo run -p strim -- -c -H 192.168.50.173 -p 8080
```

Options:
- server: `-p, --port <PORT>`, `-d, --device-id <STRING>`
- client: `-H, --host <HOST>`, `-p, --port <PORT>`
