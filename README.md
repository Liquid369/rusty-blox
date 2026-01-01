# RustyBlox - High-Performance PIVX Blockchain Explorer# rusty-blox



A lightning-fast blockchain explorer for PIVX built in Rust, featuring advanced indexing, real-time updates, and comprehensive API access.This repository contains code for parsing and processing PIVX block data. It includes functionalities for reading block files, extracting block headers and transactions, and storing the data in a database.



## ✨ Key Features## Prerequisites



- **🚀 Ultra-Fast Sync**: Leveldb-based canonical chain building + parallel blk file processingBefore running the code, make sure you have the following installed:

- **📊 Complete Indexing**: Blocks, transactions, addresses, and UTXO tracking

- **⚡ Real-Time Updates**: WebSocket support for live blockchain monitoring- Rust (version 1.51 or higher)

- **🔍 Advanced Search**: Query by block height, hash, transaction ID, or address- Cargo (Rust's package manager)

- **📈 Rich Analytics**: Network statistics, supply info, and historical charts- CMake

- **🌐 REST API**: Comprehensive v2 API with backward compatibility- Clang

- **🎨 Modern Frontend**: Responsive Vue.js interface with dark/light themes- LevelDB (dependency for reading LevelDB database files)

- RocksDB (dependency for accessing RocksDB databases)

## 🎯 Quick Start

## Installation

### Prerequisites

Clone the repository:

- Rust 1.70+ ([install here](https://rustup.rs/))

- PIVX Core node (fully synced with RPC enabled)`git clone https://github.com/Liquid369/rusty-blox.git`

- 50GB+ free disk space

- 8GB+ RAM (16GB recommended)Navigate to the project directory:



### Installation`cd rusty-blox`

Build the project using Cargo:

```bash

# Clone the repository`cargo build --release`

git clone https://github.com/liquid369/rusty-blox.git

cd rusty-blox## Usage



# Configure (copy and edit config file)The program requires a configuration file named config.toml to be present in the same directory. The configuration file specifies the paths to the block files and database files.

cp config.toml.example config.toml

# Edit config.toml with your settings## config.toml



# Build the project```[paths]

cargo build --releasedb_path = "/path/to/database.db"

blk_dir = "/path/to/block/files"

# Run the explorerldb_files_dir = "/path/to/leveldb/files"

./target/release/rustyblox```

```

To use the block parser, follow these steps:

### Configuration

Prepare the block files:

Create `config.toml` in the project root:Place your PIVX block files (e.g., blkXXXXX.dat) in a directory.

Update the blk_dir variable in the code to specify the path to the block files directory.

```tomlRun the program:

[paths]

db_path = "./data/blocks.db"`cargo run --release`

blk_dir = "/Users/liquid/Library/Application Support/PIVX/blocks"

pivx_data_dir = "/Users/liquid/Library/Application Support/PIVX"The program will process each block file in the directory, extract the block headers and transactions, and store the data in a database.

Check the output:

[sync]The program will display information about each block header and transaction as it processes the files.

fast_sync = true              # Skip UTXO tracking for faster initial sync

enrich_addresses = true       # Build address index after sync

parallel_files = 8            # Number of concurrent blk file processors## Database



[rpc]The program uses a RocksDB database to store the parsed block data. The database is created in the specified db_path directory.

host = "http://127.0.0.1:51472"

user = "explorer"## License

pass = "your_secure_password"

This project is licensed under the MIT License. See the LICENSE file for details.

[server]

host = "0.0.0.0"## Contributing

port = 3005

worker_threads = "4"Contributions are welcome! If you find any issues or have suggestions for improvements, please open an issue or submit a pull request.

```

```

### First RunPlease make sure to update the installation instructions, usage information, and license section as needed.

```

```bash
# Start the explorer
./target/release/rustyblox

# In another terminal, open the frontend
open frontend/index-enhanced.html
```

The explorer will:
1. Parse PIVX leveldb block index (~30 seconds)
2. Process all blk*.dat files in parallel (~15-20 minutes for 5M blocks)
3. Build address index (~30-50 minutes for 11.7M transactions)
4. Build transaction block index (~20-30 minutes)
5. Start live RPC monitoring

## 📚 Performance Benchmarks

| Operation | Time | Speed |
|-----------|------|-------|
| Leveldb canonical chain | 30 seconds | 5.2M blocks parsed |
| Blk file processing | 15-20 minutes | 0.43 ms/block average |
| Address enrichment | 3-5 minutes | ~1.8M tx/min |
| Transaction indexing | 2-3 minutes | ~3.8M entries/min |
| RPC catchup | ~3 minutes | 17 blocks/second (parallel) |

**Total initial sync: ~20-30 minutes** (vs hours with other explorers)

## 🏗️ Architecture

### Sync Pipeline

```
┌─────────────────────────────────────────────────────────┐
│  Phase 1: Leveldb Canonical Chain (Fast)                │
│  - Parse PIVX's block index                             │
│  - Calculate chainwork                                  │
│  - Build height→hash mappings                           │
└─────────────────────────────────────────────────────────┘
                        ↓
┌─────────────────────────────────────────────────────────┐
│  Phase 2: Blk File Processing (Parallel)                │
│  - Process 142 blk*.dat files concurrently              │
│  - Extract blocks and transactions                      │
│  - Index by canonical chain metadata                    │
└─────────────────────────────────────────────────────────┘
                        ↓
┌─────────────────────────────────────────────────────────┐
│  Phase 3: Post-Sync Enrichment (Automatic)              │
│  - Address Index: All addresses→txids                   │
│  - Block-TX Index: Fast block transaction lookups       │
└─────────────────────────────────────────────────────────┘
                        ↓
┌─────────────────────────────────────────────────────────┐
│  Phase 4: Live Monitoring (RPC)                         │
│  - Real-time block detection                            │
│  - Parallel batch processing (50 blocks/batch)          │
│  - WebSocket broadcasting                               │
└─────────────────────────────────────────────────────────┘
```

### Database Schema (RocksDB)

- **blocks**: `hash → block_header` (80 bytes)
- **transactions**: 
  - `'t' + txid → version + height + raw_tx`
  - `'B' + height + index → txid` (block transaction index)
- **chain_metadata**: 
  - `height → hash` (canonical chain)
  - `'h' + hash → height` (reverse lookup)
- **addr_index**: `'a' + address → [(txid, output_index), ...]`
- **chain_state**: `sync_height`, `network_height`, etc.

## 🔌 API Documentation

### Base URL

```
http://localhost:3005/api/v2
```

### Key Endpoints

#### Blocks

```bash
# Get block by height
GET /api/v2/block/height/:height

# Get block by hash
GET /api/v2/block/hash/:hash

# Get recent blocks
GET /api/v2/blocks/recent?limit=20

# Get block statistics
GET /api/v2/block-stats/:count
```

#### Transactions

```bash
# Get transaction
GET /api/v2/tx/:txid

# Get raw transaction hex
GET /api/v2/rawtx/:txid
```

#### Addresses

```bash
# Get address info and UTXOs
GET /api/v2/address/:address

# Get address balance
GET /api/v2/address/:address/balance
```

#### Network

```bash
# Get blockchain status
GET /api/v2/status

# Get supply information
GET /api/v2/supply
```

### WebSocket

```javascript
const ws = new WebSocket('ws://localhost:3005/ws');

ws.onmessage = (event) => {
  const data = JSON.parse(event.data);
  console.log('New block:', data);
};
```

See [API_DOCUMENTATION.md](API_DOCUMENTATION.md) for complete API reference.

## 🛠️ Development

### Project Structure

```
rusty-blox/
├── src/
│   ├── main.rs              # Entry point
│   ├── api.rs               # REST API handlers
│   ├── sync.rs              # Sync orchestration
│   ├── leveldb_index.rs     # Leveldb canonical chain
│   ├── blocks.rs            # Block file processing
│   ├── transactions.rs      # Transaction indexing
│   ├── monitor.rs           # RPC live monitoring
│   ├── enrich_addresses.rs  # Address index building
│   ├── parallel.rs          # Parallel file processing
│   └── ...
├── frontend/
│   ├── index-enhanced.html  # Modern explorer UI
│   ├── app.js               # Vue.js application
│   └── ...
├── config.toml              # Configuration
└── Cargo.toml              # Rust dependencies
```

### Building from Source

```bash
# Development build
cargo build

# Release build (optimized)
cargo build --release

# Run tests
cargo test

# Check code
cargo clippy
```

### Utility Tools

```bash
# Check database integrity
cargo run --release --bin check_db

# Rebuild transaction index
cargo run --release --bin rebuild_tx_index

# Build hash index
cargo run --release --bin build_hash_index
```

## 🚀 Deployment

See [DEPLOYMENT.md](DEPLOYMENT.md) for production deployment guide including:
- Systemd service configuration
- Nginx reverse proxy setup
- Security hardening
- Backup strategies
- Monitoring and logging

## 📖 Documentation

- [Quick Start Guide](QUICKSTART.md) - Get up and running quickly
- [API Documentation](API_DOCUMENTATION.md) - Complete API reference
- [Deployment Guide](DEPLOYMENT.md) - Production deployment
- [Frontend Guide](frontend/README.md) - Frontend development

## ⚡ Optimizations

### Fast Sync Mode

Skips UTXO and address tracking during initial sync for maximum speed:
- **Enabled** (default): ~20 minutes for 5M blocks
- **Disabled**: ~45-60 minutes (includes UTXO tracking)

Post-sync enrichment automatically runs to build address index.

### Parallel Processing

- **Blk files**: 8 concurrent processors (configurable)
- **RPC catchup**: 50 blocks per parallel batch
- **Async I/O**: Non-blocking HTTP requests

### Smart Startup

- **< 500 blocks behind**: Instant startup, RPC-only catchup
- **500-100k blocks behind**: Process last 5-10 blk files only
- **> 100k blocks behind**: Full blk file scan

## 🤝 Contributing

Contributions are welcome! Please:

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- PIVX Core team for the blockchain
- RocksDB and LevelDB teams
- Rust community

## 📬 Support

- **Issues**: [GitHub Issues](https://github.com/liquid369/rusty-blox/issues)
- **Discussions**: [GitHub Discussions](https://github.com/liquid369/rusty-blox/discussions)
- **PIVX Discord**: [Join here](https://discord.pivx.org)

---


