#!/usr/bin/env bash
# dev.sh — build Rust binaries then start vercel dev
set -e
echo "Building Rust binaries..."
cargo build
echo "Starting vercel dev..."
exec vercel dev "$@"

