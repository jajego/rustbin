# rustbin

A fast, self-hostable request bin for debugging webhooks and HTTP requests.

## Features

- **Capture HTTP requests** - Create bins to collect and inspect incoming requests
- **Real-time updates** - WebSocket support for live request monitoring  
- **Request storage** - Configurable limits with automatic cleanup
- **Rate limiting** - Built-in protection against abuse
- **SQLite storage** - No external database required
- **Zero-config** - Runs out of the box with sensible defaults

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
```

## Security

rustbin includes several security features:

- **Rate limiting** - Prevents abuse with configurable per-IP limits
- **Request size limits** - Configurable body and header size restrictions  
- **Input validation** - All inputs are validated and sanitized
- **No script execution** - Request bodies are stored as-is, no code execution
- **Automatic cleanup** - Bins expire automatically to prevent data accumulation

### Security Considerations

- **Network access**: Runs on configurable port (default 3000)
- **Data persistence**: Requests are stored in SQLite database
- **No authentication**: Public bins - suitable for non-sensitive debugging
- **CORS enabled**: Cross-origin requests allowed for webhook testing

For production deployments:
- Use reverse proxy (nginx/Caddy) with HTTPS
- Configure appropriate firewall rules
- Regular backups of the database
- Monitor resource usage and logs

## Performance & Scaling

rustbin is designed for efficient resource usage:

### Default Limits
- 100 requests per bin (configurable)
- 1MB max request body size
- 1MB max headers size  
- 2 requests/second per IP (burst of 5)
- 1 hour bin expiry time

### Resource Usage
- **Memory**: ~10-50MB base usage + request storage
- **Disk**: SQLite database grows with stored requests
- **CPU**: Minimal - async I/O with Tokio runtime

### Scaling Recommendations
- **Single instance**: Handles thousands of concurrent connections
- **Database**: SQLite suitable for most use cases; consider PostgreSQL for high load
- **Reverse proxy**: Use nginx/Caddy for TLS termination and load balancing
- **Monitoring**: Track database size, connection counts, and response times

### Tuning
- Adjust cleanup intervals for your usage patterns
- Lower bin expiry time for high-traffic scenarios  
- Increase rate limits for trusted environments
- Monitor database growth and implement archival if needed

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