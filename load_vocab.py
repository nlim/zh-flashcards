#!/usr/bin/env python3
"""
load_vocab.py — Load Chinese vocabulary from TSV into Redis.

Usage:
    REDIS_URL=redis://... python load_vocab.py

The script reads 'Chinese Vocab - Sheet1.tsv' (in the same directory),
clears any existing 'vocab' key, and pushes each valid row as a JSON
string into a Redis list under the key 'vocab'.

Install dependency first:
    pip install redis
"""

import json
import os
import sys
from pathlib import Path

try:
    import redis
except ImportError:
    sys.exit("ERROR: redis package not installed. Run: pip install redis")

# ── Config ────────────────────────────────────────────────────────────────────

TSV_FILE = Path(__file__).parent / "Chinese Vocab - Sheet1.tsv"
REDIS_KEY = "vocab"

# ── Connect ───────────────────────────────────────────────────────────────────

redis_url = os.environ.get("REDIS_URL")
if not redis_url:
    sys.exit("ERROR: REDIS_URL environment variable is not set.\n"
             "  Example: export REDIS_URL=redis://default:password@host:6379")

print(f"Connecting to Redis…")
r = redis.from_url(redis_url, decode_responses=True)

try:
    r.ping()
    print("Redis connection OK.")
except Exception as e:
    sys.exit(f"ERROR: Could not connect to Redis: {e}")

# ── Parse TSV ─────────────────────────────────────────────────────────────────

if not TSV_FILE.exists():
    sys.exit(f"ERROR: TSV file not found: {TSV_FILE}")

items = []
skipped = 0

with TSV_FILE.open(encoding="utf-8") as f:
    for lineno, line in enumerate(f, start=1):
        # Split on tab; strip trailing newline
        parts = line.rstrip("\n").split("\t")

        # Expect at least 3 columns: Pinyin, English, Characters
        if len(parts) < 3:
            skipped += 1
            continue

        pinyin     = parts[0].strip()
        english    = parts[1].strip()
        characters = parts[2].strip()

        # Skip blank rows and the header row
        if not pinyin or pinyin.lower() == "pinyin":
            skipped += 1
            continue

        # Skip rows where any required field is empty
        if not english or not characters:
            print(f"  WARNING line {lineno}: incomplete row ({pinyin!r}) — skipped")
            skipped += 1
            continue

        items.append({
            "pinyin":     pinyin,
            "english":    english,
            "characters": characters,
        })

print(f"Parsed {len(items)} vocab items ({skipped} rows skipped).")

if not items:
    sys.exit("ERROR: No valid vocab items found. Check the TSV file.")

# ── Load into Redis ───────────────────────────────────────────────────────────

print(f"Clearing existing '{REDIS_KEY}' key…")
r.delete(REDIS_KEY)

pipeline = r.pipeline()
for item in items:
    pipeline.rpush(REDIS_KEY, json.dumps(item, ensure_ascii=False))
pipeline.execute()

# Verify
count = r.llen(REDIS_KEY)
print(f"✓ Loaded {count} vocab items into Redis key '{REDIS_KEY}'.")

# Quick sanity check — print first 3 items
print("\nSample (first 3 items):")
for raw in r.lrange(REDIS_KEY, 0, 2):
    item = json.loads(raw)
    print(f"  {item['characters']}  {item['pinyin']}  →  {item['english']}")

