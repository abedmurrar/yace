#!/usr/bin/env bash
# Cut a release: verify, bump the version, roll the changelog, commit, tag.
#   scripts/release.sh 0.2.0        (or 0.2.0-rc.1)
# Then: git push origin main --follow-tags
set -euo pipefail
v="${1:?usage: release.sh VERSION (e.g. 0.2.0)}"
[[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || { echo "not a SemVer version: $v" >&2; exit 1; }
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

[[ -z "$(git status --porcelain)" ]] || { echo "working tree is not clean" >&2; exit 1; }
[[ "$(git branch --show-current)" == "main" ]] || { echo "releases are cut from main" >&2; exit 1; }
! git rev-parse -q --verify "refs/tags/v$v" >/dev/null || { echo "tag v$v already exists" >&2; exit 1; }
unreleased="$(awk '/^## \[Unreleased\]/{on=1;next} on&&/^## \[/{exit} on' CHANGELOG.md | grep -v '^\s*$' || true)"
[[ -n "$unreleased" ]] || { echo "CHANGELOG.md has nothing under [Unreleased]" >&2; exit 1; }

echo "==> checks"
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

echo "==> version $v"
python3 - "$v" <<'PY'
import re, sys, datetime
v = sys.argv[1]
s = open("Cargo.toml").read()
s = re.sub(r'(\[workspace\.package\][^\[]*?\nversion = ")[^"]+(")', rf'\g<1>{v}\2', s, count=1, flags=re.S)
open("Cargo.toml", "w").write(s)
c = open("CHANGELOG.md").read()
today = datetime.date.today().isoformat()
c = c.replace("## [Unreleased]\n", f"## [Unreleased]\n\n## [{v}] - {today}\n", 1)
m = re.search(r"^\[Unreleased\]: (.*)/compare/(v[^.]+\.[^.]+\.[^.]+\S*)\.\.\.HEAD$", c, re.M)
if m:
    base, prev = m.group(1), m.group(2)
    c = c.replace(m.group(0), f"[Unreleased]: {base}/compare/v{v}...HEAD\n[{v}]: {base}/compare/{prev}...v{v}")
open("CHANGELOG.md", "w").write(c)
# AppStream metadata (software centres show this list); stable releases only.
if "-" not in v:
    mp = "packaging/linux/io.github.abedmurrar.yace.metainfo.xml"
    x = open(mp).read()
    if f'version="{v}"' not in x:
        x = x.replace("<releases>\n", f'<releases>\n    <release version="{v}" date="{today}"/>\n', 1)
        open(mp, "w").write(x)
PY
cargo update --workspace --quiet
python3 scripts/third_party.py >/dev/null
git add Cargo.toml Cargo.lock CHANGELOG.md THIRD_PARTY_LICENSES.md packaging/linux/io.github.abedmurrar.yace.metainfo.xml
git commit -q -m "chore(release): v$v"
scripts/changelog-section.sh "$v" > /tmp/yace-tag-msg.$$
{ echo "YACE v$v"; echo; cat /tmp/yace-tag-msg.$$; } | git tag -a "v$v" -F -
rm -f /tmp/yace-tag-msg.$$
echo "==> tagged v$v. Push with: git push origin main --follow-tags"
