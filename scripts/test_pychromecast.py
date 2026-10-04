"""Discover OmniCast via mDNS and attempt a Cast TLS connect with pychromecast."""

from __future__ import annotations

import sys
import time

import pychromecast
from pychromecast.discovery import stop_discovery


FRIENDLY_NAME = "OmniCast (Laptop)"
DISCOVER_TIMEOUT = 15


def main() -> int:
    print(f"Discovering Chromecast devices (looking for '{FRIENDLY_NAME}')...")
    chromecasts, browser = pychromecast.get_listed_chromecasts(
        friendly_names=[FRIENDLY_NAME],
        discovery_timeout=DISCOVER_TIMEOUT,
    )

    if not chromecasts:
        print("No matching devices found. Listing all discovered Cast devices:")
        all_casts, all_browser = pychromecast.get_chromecasts(timeout=DISCOVER_TIMEOUT)
        try:
            if not all_casts:
                print("  (none)")
            for cast in all_casts:
                print(f"  - {cast.cast_info}")
        finally:
            stop_discovery(all_browser)
        stop_discovery(browser)
        return 1

    cast = chromecasts[0]
    print(f"Found: {cast.cast_info}")
    print("Connecting (cast.wait)...")
    try:
        cast.wait(timeout=20)
        print("Connected:", cast.cast_info)
        print("Status:", cast.status)
        print("Keeping connection open for 20s — watch OmniCast logs for Cast V2...")
        time.sleep(20)
    except Exception as exc:  # noqa: BLE001 — surface full connect failure for debugging
        print(f"Connect failed: {type(exc).__name__}: {exc}", file=sys.stderr)
        stop_discovery(browser)
        return 2
    finally:
        try:
            cast.disconnect()
        except Exception:
            pass
        stop_discovery(browser)

    print("Done.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
