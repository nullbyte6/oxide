#!/usr/bin/env bash
cargo build
sudo cp -a ./target/debug/oxide /usr/local/bin/
