import hashlib
import sys
import time
import urllib.request
from pathlib import Path

from gvsbuild.utils.base_project import Project
import gvsbuild.groups  # noqa: F401
import gvsbuild.projects  # noqa: F401
import gvsbuild.tools  # noqa: F401

MIRRORS = {
    "pixman": ["https://deb.debian.org/debian/pool/main/p/pixman/pixman_{version}.orig.tar.gz"],
    "cairo": ["https://deb.debian.org/debian/pool/main/c/cairo/cairo_{version}.orig.tar.xz"],
    "hicolor-icon-theme": [
        "https://deb.debian.org/debian/pool/main/h/hicolor-icon-theme/hicolor-icon-theme_{version}.orig.tar.xz"
    ],
    "gettext": ["https://ftp.gnu.org/gnu/gettext/gettext-{version}.tar.xz"],
    "gperf": ["https://ftp.gnu.org/gnu/gperf/gperf-{version}.tar.gz"],
    "freetype": ["https://download.savannah.gnu.org/releases/freetype/freetype-{version}.tar.xz"],
}


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def fetch(url, dest):
    request = urllib.request.Request(url, headers={"User-Agent": "fluidity-ci"})
    with urllib.request.urlopen(request, timeout=120) as response, open(dest, "wb") as out:
        while chunk := response.read(1 << 20):
            out.write(chunk)


def closure(roots):
    projects = {p.name: p for p in Project.list_projects()}
    seen, stack = {}, list(roots)
    while stack:
        name = stack.pop()
        if name in seen or name not in projects:
            continue
        seen[name] = projects[name]
        stack.extend(projects[name].dependencies)
    return seen.values()


def main():
    dest_dir = Path(sys.argv[1])
    roots = sys.argv[2:]
    dest_dir.mkdir(parents=True, exist_ok=True)
    Project.add_all()
    failed = []
    for project in closure(roots):
        url = getattr(project, "archive_url", None)
        expected = getattr(project, "hash", None)
        if not url or not expected:
            continue
        name = project.archive_filename or url[url.rfind("/") + 1 :]
        dest = dest_dir / name
        if dest.exists() and sha256(dest) == expected:
            continue
        primary = url.replace("http://", "https://", 1)
        candidates = [primary, primary, url] + [m.format(version=project.version) for m in MIRRORS.get(project.name, [])]
        for candidate in candidates:
            try:
                fetch(candidate, dest)
            except Exception as err:
                print(f"{project.name}: {candidate}: {err}")
                time.sleep(5)
                continue
            if sha256(dest) == expected:
                print(f"{project.name}: {candidate}")
                break
            print(f"{project.name}: {candidate}: hash mismatch")
        else:
            dest.unlink(missing_ok=True)
            failed.append(project.name)
    if failed:
        print("Could not fetch: " + ", ".join(failed))
        sys.exit(1)


main()
