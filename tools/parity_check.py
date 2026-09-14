"""21 フィクスチャの Java / Rust 差分を測る (カーネル外で実行する用)。

改行コードは Java 版の出力そのまま (セクション xhtml と CSS は LF、nav.xhtml と
standard.opf / toc.ncx は CRLF) を期待値とする。EPUB の中身を byte で比べるので、
作業ツリーの改行やプラットフォーム差で CRLF が混ざると差分として出る。
`--ignore-newlines` を付けたときだけ改行を無視する (以前の比較方法)。
"""

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
    sync_templates,
)

ARGS = [arg for arg in sys.argv[1:] if not arg.startswith("--")]
IGNORE_NEWLINES = "--ignore-newlines" in sys.argv
RUSTBIN = RUST / "target" / ("debug" if "--debug" in sys.argv else "release") / "AozoraEpub3_Lite.exe"
work = RUST / "target" / "audit-diff"
alld = work / "all"
del _default_binary


def entries(path):
    with zipfile.ZipFile(path) as z:
        return {n: z.read(n) for n in z.namelist()}


def compare(name, expected, actual):
    """比較用に正規化した (期待値, 実際)。タイムスタンプは毎回変わるため除外。"""
    da = re.sub(rb'dcterms:modified">[^<]*<', b'dcterms:modified"><', expected)
    db = re.sub(rb'dcterms:modified">[^<]*<', b'dcterms:modified"><', actual)
    if IGNORE_NEWLINES:
        da = da.replace(b"\r\n", b"\n")
        db = db.replace(b"\r\n", b"\n")
    return da, db


def is_newline_only(name, expected, actual):
    if name.startswith("item/image/") or name.endswith((".ttf", ".otf")):
        return False
    ea, eb = compare(name, expected, actual)
    return (
        ea != eb
        and ea.replace(b"\r\n", b"\n") == eb.replace(b"\r\n", b"\n")
        and expected != actual
    )


sync_tables()
sync_templates()

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
                da, db = compare(n, a[n], b[n])
                if da != db:
                    short = n.replace("item/", "")
                    if is_newline_only(n, a[n], b[n]):
                        diffs.append(f"{short} (改行のみ)")
                    else:
                        diffs.append(short)
    if not diffs:
        clean += 1
    print(f"{'OK  ' if not diffs else 'DIFF'} {stem}: {diffs if diffs else ''}", flush=True)
print(f"clean: {clean}/{len(fixtures)}")
