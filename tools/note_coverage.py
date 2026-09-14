"""注記表 (chuki_tag.txt / chuki_tag_suf.txt) の全行を 1 行ずつ変換して
Java 版と Rust 版の出力を突き合わせる。

使い方 (リポジトリ直下で):
    python tools/note_coverage.py [--only 柱] [--limit N]
"""

import argparse
import difflib
import pathlib
import re
import shutil
import subprocess
import sys
import zipfile

RUST = pathlib.Path(__file__).resolve().parent.parent
work = RUST / "target" / "note-coverage"
classes = RUST / "target" / "audit-diff" / "classes"
JAR = pathlib.Path("C:/Users/rumia/Documents/AozoraEpub3/AozoraEpub3.jar")
jdir = RUST / "target" / "audit-diff" / "javabin"
JAVA_REPO = pathlib.Path("C:/Users/rumia/Desktop/APP/Java/AozoraEpub3")
RUSTBIN = RUST / "target" / "release" / "AozoraEpub3_Lite.exe"


def run(cmd, cwd):
    p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True,
                       encoding="utf-8", errors="replace")
    return p.returncode, (p.stdout or "") + (p.stderr or "")


def tag_rows():
    """chuki_tag.txt の (見出し, 注記テキスト) 一覧。

    開始注記と終わり注記は対にして 1 ケースにする (片方だけの行は単体で出す)。
    """
    notes = []
    for line in (JAVA_REPO / "chuki_tag.txt").read_text("utf-8", errors="replace").splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        fields = line.split("\t")
        note = fields[0].strip()
        if note:
            notes.append(note)
    known = set(notes)

    def close_of(note):
        if note.endswith("終わり"):
            return None
        candidates = []
        if note.startswith("ここから"):
            candidates.append("ここまで" + note[len("ここから"):])
            candidates.append("ここで" + note[len("ここから"):] + "終わり")
        candidates.append(note + "終わり")
        candidates.append(note + "終り")
        for candidate in candidates:
            if candidate in known:
                return candidate
        return None

    rows = []
    for note in notes:
        if note.endswith(("終わり", "終り")):
            # 対応する開始注記があればそちらで検証する
            continue
        if note.startswith("ここまで"):
            continue
        close = close_of(note)
        body = f"［＃{note}］テスト本文" + (f"［＃{close}］" if close else "")
        rows.append((f"tag-{len(rows):04}", body))
    return rows


def suffix_rows():
    """chuki_tag_suf.txt の (見出し, 注記テキスト) 一覧。"""
    rows = []
    for line in (JAVA_REPO / "chuki_tag_suf.txt").read_text("utf-8", errors="replace").splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        fields = line.split("\t")
        suffix = fields[0].strip()
        if suffix:
            # 実際の用法どおり対象文字列の後ろに置く
            rows.append((f"suf-{len(rows):04}", f"テスト対象本文［＃「テスト対象本文」{suffix}］"))
    return rows


def entries(path):
    with zipfile.ZipFile(path) as z:
        return {n: z.read(n).replace(b"\r\n", b"\n").decode("utf-8", "replace")
                for n in z.namelist()}


def body_of(epub):
    """dcterms 等の時刻依存部分を除いた XHTML 全体。"""
    text = epub.get("item/xhtml/0001.xhtml", "")
    return re.sub(r'dcterms:modified">[^<]*<', 'dcterms:modified"><', text)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--only", default=None)
    parser.add_argument("--limit", type=int, default=0)
    args = parser.parse_args()

    rows = tag_rows() + suffix_rows()
    if args.only:
        rows = [r for r in rows if args.only in r[1]]
    if args.limit:
        rows = rows[: args.limit]
    print(f"notes: {len(rows)}", flush=True)

    shutil.rmtree(work, ignore_errors=True)
    src = work / "src"
    src.mkdir(parents=True)
    paths = []
    for key, note in rows:
        path = src / f"{key}.txt"
        path.write_text(f"表題{key}\n著者{key}\n\n{note}テスト本文\n", encoding="cp932")
        paths.append(str(path))

    jd = work / "j"
    rd = work / "r"
    jd.mkdir()
    rd.mkdir()
    # コマンドライン長の制限があるので分割して 1 プロセスずつ回す
    chunk_size = 80
    for start in range(0, len(paths), chunk_size):
        chunk = paths[start:start + chunk_size]
        run(["java", "-cp", f"{classes};{JAR}", "AozoraEpub3", "-ext", ".epub",
             "-d", str(jd), *chunk], jdir)
        rc, log = run([str(RUSTBIN), "-ext", ".epub", "-d", str(rd), *chunk], RUST)
        if rc != 0:
            print("RUST EXIT", rc, log[:300], "at", chunk[0])
            return 1

    java_out = {p.name: entries(p) for p in jd.glob("*.epub")}
    rust_out = {p.name: entries(p) for p in rd.glob("*.epub")}
    missing = sorted(set(java_out) - set(rust_out))
    extra = sorted(set(rust_out) - set(java_out))
    if missing:
        print("only java produced:", missing[:10])
    if extra:
        print("only rust produced:", extra[:10])

    by_key = dict(rows)
    tag_tags = {}
    for line in (JAVA_REPO / "chuki_tag.txt").read_text("utf-8", errors="replace").splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        fields = line.split("\t")
        note = fields[0].strip()
        if note:
            tag_tags[note] = [f.strip() for f in fields[1:]
                              if f.strip() and f.strip() not in ("1", "2", "3", "P", "M", "L", "K")]
    differs = []
    for name in sorted(set(java_out) & set(rust_out)):
        jb = body_of(java_out[name])
        rb = body_of(rust_out[name])
        if jb != rb:
            key = re.search(r"(tag|suf)-\d+", name)
            differs.append((key.group(0) if key else name, jb, rb))
    print(f"differs: {len(differs)}/{len(java_out)}", flush=True)
    gaps = []
    for key, jb, rb in differs:
        note = by_key.get(key, "")
        fragments = []
        for text in tag_tags.get(note, []):
            text = re.sub(r"%[sd]", "", text).strip()
            if not text:
                continue
            name_match = re.match(r"</?([a-zA-Z0-9]+)", text)
            if not name_match:
                continue
            class_match = re.search(r'class="([^"]*)"', text)
            fragment = (f'class="{class_match.group(1).strip()}"' if class_match
                        else (f"<{name_match.group(1)}" if not text.startswith("</")
                              else f"</{name_match.group(1)}>"))
            if fragment in jb and fragment not in rb:
                fragments.append(fragment)
        if fragments:
            gaps.append((key, note, sorted(set(fragments))))
    print(f"\n=== Java が出すタグを Rust が出していない注記: {len(gaps)} ===")
    for key, note, fragments in gaps:
        print(f"  [{key}] ［＃{note}］ missing={fragments}")

    for key, jb, rb in differs:
        parts = []
        for line in difflib.unified_diff(jb.splitlines(), rb.splitlines(), "j", "r",
                                         lineterm="", n=0):
            if line.startswith(("+", "-")) and not line.startswith(("+++", "---")):
                parts.append(line.strip()[:120])
        print(f"  [{key}] {by_key.get(key, '?')}")
        for part in parts[:4]:
            print(f"      {part}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
