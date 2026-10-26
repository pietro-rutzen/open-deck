"""Read-only BPM microbenchmark against the latest workspace build artifacts."""
import glob
import os
import sqlite3
import subprocess
import tempfile
from pathlib import Path

repo = Path(__file__).resolve().parents[2]
with tempfile.TemporaryDirectory(prefix="opendeck-benchmark-") as scratch:
    executable = str(Path(scratch) / "bpm-benchmark")
    args = ["rustc", "--edition=2021", "-C", "opt-level=0", str(repo / "docs/testing/bpm-benchmark.rs"), "-L", "dependency=" + str(repo / "target/debug/deps")]
    for name in ["compas_sources", "compas_dsp"]:
        matches = glob.glob(str(repo / "target/debug/deps") + "/lib" + name + "-*.rlib")
        if not matches:
            raise SystemExit("Build the prototype first.")
        artifact = max(matches, key=os.path.getmtime)
        print(name + ": " + os.path.basename(artifact), flush=True)
        args += ["--extern", name + "=" + artifact]
    subprocess.run(args + ["-o", executable], check=True)
    database = Path.home() / "Library/Application Support/org.opendeck.prototype/compas.db"
    paths = []
    if database.exists():
        with sqlite3.connect(database.as_uri() + "?mode=ro", uri=True) as connection:
            paths = [row[0] for row in connection.execute("SELECT path FROM tracks LIMIT 3")]
    fixtures = [repo / "samples/compas-test-120bpm.wav"]
    fixtures = [str(path) for path in fixtures if path.exists()]
    subprocess.run([executable] + fixtures + paths, check=True)
