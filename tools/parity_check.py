"""21 フィクスチャの Java / Rust 差分を測る (カーネル外で実行する用)。"""

import pathlib
import re
import shutil
import sys
import zipfile

from java_reference import (
    CLASSES as classes,
    JAR,
    JAVABIN as jdir,
    RUST,
    RUSTBIN as _default_binary,
    run,
    sync_tables,
)

RUSTBIN = RUST / "target" / ("debug" if "--debug" in sys.argv else "release") / "AozoraEpub3_Lite.exe"
work = RUST / "target" / "audit-diff"
alld = work / "all"
del _default_binary


def entries(path):
    with zipfile.ZipFile(path) as z:
        return {n: z.read(n).replace(b"\r\n", b"\n").decode("utf-8", "replace")
                for n in z.namelist()}


sync_tables()

fixtures = sorted(str(p) for p in alld.glob("*.txt")) + sorted(str(p) for p in alld.glob("*.zip"))
base = RUST / "target" / "fx-run"
shutil.rmtree(base, ignore_errors=True)
base.mkdir(parents=True)
clean = 0
for fx in fixtures:
    stem = pathlib.Path(fx).stem
    jd = base / f"{stem}-j"
    rd = base / f"{stem}-r"
    jd.mkdir(); rd.mkdir()
    run(["java", "-cp", f"{classes};{JAR}", "AozoraEpub3", "-ext", ".epub",
         "-d", str(jd), fx], jdir)
    rc, _ = run([str(RUSTBIN), "-ext", ".epub", "-d", str(rd), fx], RUST)
    jf = sorted(jd.glob("*.epub")); rf = sorted(rd.glob("*.epub"))
    diffs = []
    if rc != 0:
        diffs.append(f"RUST EXIT {rc}")
    if [p.name for p in jf] != [p.name for p in rf]:
        diffs.append(f"name java={[p.name for p in jf]} rust={[p.name for p in rf]}")
    else:
        for jp, rp in zip(jf, rf):
            a, b = entries(jp), entries(rp)
            only_j = sorted(set(a) - set(b)); only_r = sorted(set(b) - set(a))
            if only_j:
                diffs.append(f"only-java:{only_j}")
            if only_r:
                diffs.append(f"only-rust:{only_r}")
            for n in sorted(set(a) & set(b)):
                da = re.sub(r'dcterms:modified">[^<]*<', 'dcterms:modified"><', a[n])
                db = re.sub(r'dcterms:modified">[^<]*<', 'dcterms:modified"><', b[n])
                if da != db:
                    diffs.append(n.replace("item/", ""))
    if not diffs:
        clean += 1
    print(f"{'OK  ' if not diffs else 'DIFF'} {stem}: {diffs if diffs else ''}", flush=True)
print(f"clean: {clean}/{len(fixtures)}")
