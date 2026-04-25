#!/bin/bash
cargo build --release 2>&1 | tail -20
./target/release/rustyblox --reset-height-resolution 2>&1 | head -20
