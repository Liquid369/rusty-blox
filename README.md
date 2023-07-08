# rusty-blox

This repository contains code for parsing and processing PIVX block data. It includes functionalities for reading block files, extracting block headers and transactions, and storing the data in a database.

## Prerequisites

Before running the code, make sure you have the following installed:

- Rust (Programming language)
- RocksDB (Database)

## Installation

Clone the repository:

`git clone https://github.com/Liquid369/rusty-blox.git`

Navigate to the project directory:

`cd rusty-blox`
Build the project using Cargo:

`cargo build --release`

## Usage

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
