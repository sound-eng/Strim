# WARP.md

This file provides guidance to WARP (warp.dev) when working with code in this repository.

## Project Overview

Strim is a pair of CLI applications (client and server) for real-time audio streaming over TCP. It's an exercise project demonstrating audio capture, network streaming, and audio playback in Rust.

**Key Components:**
- **strim-server**: Captures audio from the default input device and streams it to connected clients
- **strim-client**: Connects to the server and plays received audio through the default output device  
- **strim-shared**: Common types, protocols, and utilities used by both client and server

## Architecture

### Network Protocol
The applications use a custom binary protocol over TCP with length-prefixed messages:
- `Message::Config(AudioConfig)` - Sent by server to configure client audio parameters
- `Message::AudioData(Vec<u8>)` - Raw audio samples streamed from server to client
- `Message::Error(String)` - Error notifications between client and server

### Audio Pipeline
1. **Server**: CPAL audio capture → Sample conversion → TCP transmission
2. **Client**: TCP reception → Message deserialization → CPAL audio playback

The system supports multiple sample formats (I16, I32, F32, U16) with automatic format conversion between CPAL and the shared protocol.

### Threading Model
**Server threads:**
- Main thread: Coordinates shutdown and keeps process alive
- Audio capture thread: CPAL stream callback sending to broadcast channel
- Client accept thread: Handles new TCP connections  
- Broadcast thread: Distributes audio data to all connected clients
- Health check thread: Removes disconnected clients

**Client threads:**
- Main thread: Manages connection lifecycle
- Connection manager thread: Handles connection/reconnection logic
- Network read thread: Receives and deserializes messages from server
- Audio playback thread: CPAL output stream

## Development Commands

### Build and Check
```bash
# Check all packages for compilation errors
cargo check --workspace

# Build all binaries
cargo build --workspace

# Build with optimizations
cargo build --workspace --release
```

### Running the Applications
```bash
# Start the server (default port 8080)
cargo run --bin strim-server

# Start server on custom port
cargo run --bin strim-server -- --port 9000

# Connect client to localhost:8080
cargo run --bin strim-client

# Connect client to custom host/port  
cargo run --bin strim-client -- --host 192.168.1.100 --port 9000
```

### Development Workflow
```bash
# Run individual package checks
cargo check -p strim-server
cargo check -p strim-client  
cargo check -p strim-shared

# Format code
cargo fmt --all

# Run linter
cargo clippy --workspace -- -D warnings

# Build specific binary
cargo build --bin strim-server
cargo build --bin strim-client
```

### Testing
The project has a comprehensive test suite covering all components:

```bash
# Run all tests (unit, integration, and library tests)
cargo test --workspace

# Run tests for individual packages
cargo test -p strim-shared    # Message serialization, AudioSample trait, format conversions
cargo test -p strim-server    # Audio processing, client management, network broadcasting
cargo test -p strim-client    # Connection management, audio playback, retry logic

# Run integration tests (network protocol verification)
cargo test -p strim-shared --test integration_tests

# Run tests with output
cargo test --workspace -- --nocapture
```

**Test Coverage:**
- **Unit Tests**: Message serialization/deserialization, audio sample format handling, network protocol components
- **Integration Tests**: End-to-end client-server communication, large message handling, connection lifecycle
- **Functional Tests**: Audio processing pipeline, connection retry logic, broadcast mechanisms

## Important Implementation Details

### Audio Sample Handling
The `AudioSample` trait in `strim-shared` provides generic sample format handling. When adding new sample formats, implement this trait for type-safe byte conversion.

### Graceful Shutdown
Both applications handle Ctrl+C gracefully:
- Server notifies all clients before shutdown
- Client attempts clean disconnection and reconnects automatically on connection loss

### Network Reliability
- Client implements automatic reconnection with retry logic
- Server uses health checks to detect and remove disconnected clients
- TCP nodelay is enabled for low-latency streaming

## Known Issues

1. **Unused Variables**: Several warnings about unused variables in the client code (current_stream, unreachable pattern)
2. **Dead Code**: The ConnectionEvent::Shutdown variant is never constructed according to compiler warnings

## Network Configuration

- **Default Port**: 8080 (configurable via CLI)  
- **Protocol**: TCP with custom binary message format
- **Connection**: Server binds to 0.0.0.0, client connects to localhost by default
- **Buffer Size**: 4KB audio chunks (defined in `DEFAULT_BUFFER_SIZE`)