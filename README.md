# rusty-blox

This repository contains code for parsing and processing PIVX block data. It includes functionalities for reading block files, extracting block headers and transactions, and storing the data in a database.

## Prerequisites

Before running the code, make sure you have the following installed:

- Rust (version 1.51 or higher)
- Cargo (Rust's package manager)
- CMake
- Clang
- LevelDB (dependency for reading LevelDB database files)
- RocksDB (dependency for accessing RocksDB databases)

## Installation

Clone the repository:

`git clone https://github.com/Liquid369/rusty-blox.git`

Navigate to the project directory:

`cd rusty-blox`
Build the project using Cargo:

`cargo build --release`

## Usage

The program requires a configuration file named config.toml to be present in the same directory. The configuration file specifies the paths to the block files and database files.

## config.toml

```[paths]
db_path = "/path/to/database.db"
blk_dir = "/path/to/block/files"
ldb_files_dir = "/path/to/leveldb/files"
```

To use the block parser, follow these steps:

Prepare the block files:
Place your PIVX block files (e.g., blkXXXXX.dat) in a directory.
Update the blk_dir variable in the code to specify the path to the block files directory.
Run the program:

`cargo run --release`

The program will process each block file in the directory, extract the block headers and transactions, and store the data in a database.
Check the output:
The program will display information about each block header and transaction as it processes the files.


## Database

The program uses a RocksDB database to store the parsed block data. The database is created in the specified db_path directory.

## License

This project is licensed under the MIT License. See the LICENSE file for details.

## Contributing

Contributions are welcome! If you find any issues or have suggestions for improvements, please open an issue or submit a pull request.

```
Please make sure to update the installation instructions, usage information, and license section as needed.
```
