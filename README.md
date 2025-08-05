# rustbin

A fast, self-hostable request bin for debugging webhooks and HTTP requests.

rustbin is accessible online at https://rustb.in, or read below to deploy it yourself.

## Features

- **Capture HTTP requests** - Create bins to collect and inspect incoming requests
- **Real-time updates** - WebSocket support for live request monitoring  
- **Request storage** - Configurable limits with automatic cleanup
- **SQLite storage** - No external database required

## Quick Start

```bash
# Clone and run
git clone https://github.com/jajego/rustbin.git
cd rustbin
cargo run

# Server starts at http://localhost:3000
# Frontend available in rustbin-frontend/
```

## Self-Hosting

### Docker (Recommended)

```bash
# Build and run
docker build -t rustbin .
docker run -p 3000:3000 -v rustbin-data:/app/data rustbin
```

### Manual Installation

```bash
# Install Rust if needed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build release binary
cargo build --release
./target/release/rustbin

# Configure via rustbin.toml (auto-generated on first run)
```

## Configuration

Edit `rustbin.toml` to customize settings:

```toml
[server]
host = "0.0.0.0"    # Bind address
port = 3000         # Port number

[database]
url = "sqlite://rustbin.db"  # Database path
max_connections = 5          # Connection pool size

[rate_limiting]
requests_per_second = 2  # Rate limit per IP
burst_size = 5          # Burst allowance

[limits]
max_requests_per_bin = 100    # Requests stored per bin
max_body_size = 1048576      # Max request body (1MB)
max_headers_size = 1048576   # Max headers size (1MB)

[cleanup]
bin_expiry_hours = 1         # Auto-delete inactive bins
cleanup_interval_seconds = 60 # Cleanup frequency
```

## API

### Create a bin
```bash
curl -X POST http://localhost:3000/create
# Returns: {"id": "bin-uuid", "url": "http://localhost:3000/bin/bin-uuid"}
```

### Send requests to bin
```bash
curl -X POST http://localhost:3000/bin/{bin-id} \
  -H "Content-Type: application/json" \
  -d '{"test": "data"}'
```

### Inspect bin requests
```bash
curl http://localhost:3000/bin/{bin-id}/inspect
```

### WebSocket monitoring
```javascript
const ws = new WebSocket('ws://localhost:3000/bin/{bin-id}/ws');
ws.onmessage = (event) => console.log(JSON.parse(event.data));

## Development

```bash
# Run tests
cargo test

# Run with live reload
cargo install cargo-watch
cargo watch -x run

# Frontend development
cd rustbin-frontend
npm install
npm run dev

# Format code
cargo fmt
cd rustbin-frontend && npm run format

# Lint code
cargo clippy
cd rustbin-frontend && npm run lint
```

## License

MIT
