#!/usr/bin/env python3
import argparse
import hashlib
import io
import re
import sys
import tarfile
import urllib.request
from pathlib import Path

TWEMOJI_TAG = "v17.0.3"
TWEMOJI_COMMIT = "b6b55fef1e8636b540a6d016a4729ca8cdf2e60b"
ARCHIVE_URL = f"https://codeload.github.com/jdecked/twemoji/tar.gz/{TWEMOJI_COMMIT}"
ARCHIVE_SHA256 = "705d79de1460e5e775f362f0d0f01fbe3ef8d65bf4648c490e4649704584f747"
LIST_SIZE = 256

ROOT = Path(__file__).resolve().parents[2]
LIST = ROOT / "web" / "emoji" / "list.txt"
SPRITE = ROOT / "web" / "emoji.svg"

SVG_RE = re.compile(r'^<svg xmlns="http://www\.w3\.org/2000/svg" viewBox="0 0 (\d+(?:\.\d+)?) (\d+(?:\.\d+)?)"(?: xml:space="preserve")?>(.*)</svg>$', re.S)
ATTRIBUTION = f"""<!--
Twemoji graphics by Twitter, Inc and other contributors, licensed under CC-BY 4.0:
https://creativecommons.org/licenses/by/4.0/
Source: https://github.com/jdecked/twemoji {TWEMOJI_TAG} ({TWEMOJI_COMMIT}), assets/svg.
Combined into one sprite (one symbol per file) by packaging/twemoji/build-sprite.py.
-->"""
CODEPOINT_RE = re.compile(r"^[0-9a-f]{4,5}$")


def read_list():
    ids = []
    for n, line in enumerate(LIST.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        cp = line.split()[0]
        if not CODEPOINT_RE.match(cp):
            sys.exit(f"{LIST}:{n}: not a single codepoint: {cp}")
        ids.append(cp)
    if len(ids) != LIST_SIZE or len(set(ids)) != LIST_SIZE:
        sys.exit(f"{LIST}: need {LIST_SIZE} distinct entries, found {len(ids)} ({len(set(ids))} distinct)")
    return ids


def fetch_archive(path):
    if path:
        data = Path(path).read_bytes()
    else:
        with urllib.request.urlopen(ARCHIVE_URL) as res:
            data = res.read()
    digest = hashlib.sha256(data).hexdigest()
    if digest != ARCHIVE_SHA256:
        sys.exit(f"archive sha256 {digest} does not match {ARCHIVE_SHA256}")
    return data


def read_svgs(data, ids):
    wanted = {f"{cp}.svg": cp for cp in ids}
    found = {}
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tar:
        for member in tar:
            parts = member.name.split("/")
            if member.isfile() and parts[1:3] == ["assets", "svg"] and parts[-1] in wanted:
                found[wanted[parts[-1]]] = tar.extractfile(member).read().decode("utf-8")
    missing = [cp for cp in ids if cp not in found]
    if missing:
        sys.exit(f"missing in the archive: {' '.join(missing)}")
    return found


def symbol(cp, svg):
    m = SVG_RE.match(svg.strip())
    if not m:
        sys.exit(f"{cp}.svg: unexpected root element")
    width, height, body = float(m.group(1)), float(m.group(2)), m.group(3)
    if width != 36 or height > 36:
        sys.exit(f"{cp}.svg: unexpected viewBox 0 0 {width:g} {height:g}")
    if re.search(r"\bid=|url\(#|href=", body):
        sys.exit(f"{cp}.svg: uses ids or references; pick another emoji or rewrite them here")
    if height < 36:
        body = f'<g transform="translate(0 {(36 - height) / 2:g})">{body}</g>'
    return f'<symbol id="e-{cp}" viewBox="0 0 36 36">{body}</symbol>'


def main():
    parser = argparse.ArgumentParser(description=f"Build web/emoji.svg from Twemoji {TWEMOJI_TAG}.")
    parser.add_argument("--archive", help="use a downloaded copy of the pinned archive instead of fetching it")
    args = parser.parse_args()
    ids = read_list()
    svgs = read_svgs(fetch_archive(args.archive), ids)
    lines = [ATTRIBUTION, '<svg xmlns="http://www.w3.org/2000/svg">']
    lines += [symbol(cp, svgs[cp]) for cp in ids]
    lines.append("</svg>")
    SPRITE.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {SPRITE} ({SPRITE.stat().st_size} bytes, {len(ids)} symbols)")


if __name__ == "__main__":
    main()
