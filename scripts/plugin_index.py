#!/usr/bin/env python3
"""Maintain the plugin index: site/src/plugins/index.json.

YACE reads it (Extensions: Browse Plugins…, `yace --install-plugin <id>`)
from https://abedmurrar.github.io/yace/plugins/index.json. The list of
plugins on site/src/plugins/using.md is generated from it.

  scripts/plugin_index.py add URL [--target linux-x86_64]
      Download a released package, read its manifest, and add or update
      its entry (a new version replaces every package of the old one).
  scripts/plugin_index.py check [--offline]
      Validate the index and the generated list; without --offline also
      download every package and check its SHA-256, id and version.
"""

import argparse
import hashlib
import io
import json
import re
import sys
import tarfile
import tomllib
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
INDEX = ROOT / "site/src/plugins/index.json"
PAGE = ROOT / "site/src/plugins/using.md"
START, END = "<!-- plugin-index:start -->", "<!-- plugin-index:end -->"
FORMAT = 1
ID = re.compile(r"^[a-z0-9_][a-z0-9_-]*(\.[a-z0-9_][a-z0-9_-]*)+$")
MAX_BYTES = 256 << 20


def download(url):
    with urllib.request.urlopen(url, timeout=120) as r:
        data = r.read(MAX_BYTES + 1)
    if len(data) > MAX_BYTES:
        sys.exit(f"{url}: larger than {MAX_BYTES} bytes")
    return data


def manifest_of(data):
    """The manifest of a package: at its root or in one top-level folder."""
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tar:
        names = [m.name for m in tar.getmembers() if m.isfile()]
        for name in sorted(names, key=len):
            if name.removeprefix("./").count("/") <= 1 and name.endswith("yace-plugin.toml"):
                return tomllib.loads(tar.extractfile(name).read().decode())
    sys.exit("the package has no yace-plugin.toml")


def load():
    return json.loads(INDEX.read_text()) if INDEX.exists() else {"format": FORMAT, "plugins": []}


def save(index):
    index["plugins"].sort(key=lambda e: e["id"])
    INDEX.write_text(json.dumps(index, indent=2, ensure_ascii=False) + "\n")
    text = PAGE.read_text()
    a, b = text.index(START) + len(START), text.index(END)
    PAGE.write_text(text[:a] + "\n" + listing(index) + text[b:])


def listing(index):
    lines = []
    for e in sorted(index["plugins"], key=lambda e: e["name"].lower()):
        name = f"[{e['name']}]({e['repository']})" if e.get("repository") else e["name"]
        kind = "Native code" if e.get("code") else "Data only"
        desc = e.get("description", "").rstrip(".")
        lines.append(f"| {name} | {desc}. {kind}. | `yace --install-plugin {e['id']}` |")
    if not lines:
        return "No plugins are listed yet.\n"
    return "| Plugin | What it is | Install |\n|---|---|---|\n" + "\n".join(lines) + "\n"


def add(url, target):
    if not url.startswith("https://"):
        sys.exit("the package URL must use https")
    data = download(url)
    m = manifest_of(data)
    sha = hashlib.sha256(data).hexdigest()
    index = load()
    old = next((e for e in index["plugins"] if e["id"] == m["id"]), None)
    packages = []
    if old and old["version"] == m["version"]:
        packages = [p for p in old["packages"] if p["target"] != target]
    packages.append({"target": target, "url": url, "sha256": sha})
    entry = {
        "id": m["id"],
        "name": m["name"],
        "version": m["version"],
        "description": m.get("description", ""),
        "publisher": m.get("publisher", ""),
        "license": m.get("license", ""),
        "repository": m.get("homepage", ""),
        "yace": m.get("yace", "*"),
        "code": "library" in m,
        "packages": sorted(packages, key=lambda p: p["target"]),
    }
    index["plugins"] = [e for e in index["plugins"] if e["id"] != m["id"]] + [entry]
    save(index)
    print(f"{'Updated' if old else 'Added'} {m['id']} {m['version']} ({target}, sha256 {sha[:16]}…)")


def check(offline):
    index = load()
    problems = []
    if index.get("format") != FORMAT:
        problems.append(f"format must be {FORMAT}")
    seen = set()
    for e in index.get("plugins", []):
        where = e.get("id", "?")
        if not ID.match(e.get("id", "")):
            problems.append(f"{where}: invalid id")
        if where in seen:
            problems.append(f"{where}: listed twice")
        seen.add(where)
        if not e.get("packages"):
            problems.append(f"{where}: no packages")
        for p in e.get("packages", []):
            if not p["url"].startswith("https://"):
                problems.append(f"{where}: {p['url']} is not https")
            if not re.fullmatch(r"[0-9a-f]{64}", p["sha256"]):
                problems.append(f"{where}: bad sha256")
            if offline:
                continue
            data = download(p["url"])
            if hashlib.sha256(data).hexdigest() != p["sha256"]:
                problems.append(f"{where}: {p['url']} doesn't match its sha256")
                continue
            m = manifest_of(data)
            if (m["id"], m["version"]) != (e["id"], e["version"]):
                problems.append(f"{where}: the package is {m['id']} {m['version']}, the index says {e['version']}")
    text = PAGE.read_text()
    shown = text[text.index(START) + len(START) : text.index(END)]
    if shown != "\n" + listing(index):
        problems.append(f"{PAGE.relative_to(ROOT)}: the plugin list is out of date (run `add` again or `save`)")
    for p in problems:
        print(f"plugin index: {p}", file=sys.stderr)
    if problems:
        sys.exit(1)
    print(f"plugin index: {len(index['plugins'])} plugin(s) OK" + (" (offline)" if offline else ""))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    a = sub.add_parser("add")
    a.add_argument("url")
    a.add_argument("--target", default="any", help="any (data only) or os-arch, e.g. linux-x86_64")
    c = sub.add_parser("check")
    c.add_argument("--offline", action="store_true")
    args = ap.parse_args()
    if args.cmd == "add":
        add(args.url, args.target)
    else:
        check(args.offline)


if __name__ == "__main__":
    main()
