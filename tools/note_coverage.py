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
import sys
import zipfile

from java_reference import (
    CLASSES as classes,
    JAR,
    JAVABIN as jdir,
    RUST,
    RUSTBIN,
    run,
    suffix_table_text,
    sync_tables,
    tag_table_text,
)

work = RUST / "target" / "note-coverage"


def _tag_notes():
    notes = []
    for line in tag_table_text().splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        note = line.split("\t")[0].strip()
        if note:
            notes.append(note)
    return notes


def close_of(note, notes):
    """開始注記に対応する終了注記を表の意味論から求める。

    `X前`↔`X後`、`ここからX`↔`ここまでX`/`ここでX終わり` のほか、
    段番号が落ちる綴り (`ここから１段階大きな文字` ↔ `ここで大きな文字終わり`)
    を接尾辞の最長一致で拾う。
    """
    known = set(notes)
    if note.endswith(("終わり", "終り")):
        return None
    if note.endswith("前") and note[:-1] + "後" in known:
        return note[:-1] + "後"
    if note.endswith("開始") and note[:-2] + "終了" in known:
        return note[:-2] + "終了"
    if note.startswith("ここから"):
        rest = note[len("ここから"):]
        for candidate in (
            "ここまで" + rest,
            f"ここで{rest}終わり",
            f"ここで{rest}終り",
        ):
            if candidate in known:
                return candidate
    for candidate in (note + "終わり", note + "終り"):
        if candidate in known:
            return candidate
    # 表記が一部落ちる綴りを接尾辞の最長一致で拾う。
    # `ここから…` は `ここで…終わり`/`ここまで…` で閉じ、それ以外は
    # `…終わり` で閉じる (取り違えるとブロック用の `</div>` を当ててしまう)。
    body = note[len("ここから"):] if note.startswith("ここから") else note
    block_form = note.startswith("ここから")
    candidates = []
    for candidate in notes:
        if candidate.startswith("ここまで"):
            rest = candidate[len("ここまで"):]
        elif candidate.startswith("ここで") and candidate.endswith(("終わり", "終り")):
            rest = candidate[len("ここで"):]
            rest = rest[:-3] if rest.endswith("終わり") else rest[:-2]
        elif candidate.endswith(("終わり", "終り")):
            rest = candidate[:-3] if candidate.endswith("終わり") else candidate[:-2]
        else:
            continue
        is_block_form = candidate.startswith(("ここまで", "ここで"))
        if is_block_form != block_form:
            continue
        if rest and body.endswith(rest):
            candidates.append((len(rest), candidate))
    if candidates:
        return max(candidates)[1]
    return None


# Narou Bridge のリンク注記は 3 点 1 組 (ｌｉｎｋ＿ｓ + URL + ｌｉｎｋ＿ｔ + 文言 +
# ｌｉｎｋ＿ｅ) で 1 つの `<a>` になる。断片側 (ｓ/ｔ) は不完全タグとして除外されるが、
# ｅ (`</a>`) は完全タグのため単独ケース化されて「Java は裸の </a> を出す」という
# 偽ギャップになる。実用法は realistic_cases.py の `narou-link` で検証する。
LINK_ROW_PATTERN = re.compile("^(ｌｉｎｋ＿[ｓｔｅ]|link_[ste])$")


def is_internal_row(fields):
    """利用者が本文に書けない行かどうか。

    - 画像タグ 19 行: Java `printImageChuki` が `String.format` で埋める内部
      テンプレート (画像注記から生成される)。本文に直接書いても意味を持たない。
    - 折り返し1/2/3 等の断片・属性 14 行: 複合タグの一部で単独では不完全な HTML。
    """
    note = fields[0].strip()
    tag = fields[1].strip() if len(fields) > 1 else ""
    close_tag = fields[2].strip() if len(fields) > 2 else ""
    if LINK_ROW_PATTERN.match(note):
        return True
    if "%" in tag or "%" in close_tag:
        return True
    if tag and not tag.startswith("%"):
        if ("<" in tag) != (">" in tag):
            return True
        # 素の属性・クラス断片 (border / dashed_border / center / yoko / idt / jzm)
        # は複合字下げ専用。`&#8203;` のような実体参照は通常の注記なので残す。
        if "<" not in tag and ">" not in tag and re.fullmatch(r"[A-Za-z_][A-Za-z_ -]*", tag):
            return True
    del note
    return False


def tag_rows():
    """chuki_tag.txt の (見出し, 注記テキスト) 一覧。

    開始注記と終了注記は意味論的に対にして 1 ケースにする。対になる注記が
    無い開始注記は単独で、終了専用の注記は対の開始注記側で検証する。
    """
    all_fields = [
        line.split("\t")
        for line in tag_table_text().splitlines()
        if line.strip() and not line.startswith("#")
    ]
    internal = [f for f in all_fields if is_internal_row(f)]
    notes = [f[0].strip() for f in all_fields
             if f[0].strip() and not is_internal_row(f)]
    print(f"  (内部生成タグ/断片として除外: {len(internal)})", flush=True)
    known = set(notes)
    closers = {close_of(note, notes) for note in notes}
    closers.discard(None)
    rows = []
    unclosed = []
    for note in notes:
        if note.endswith(("終わり", "終り")) or note.startswith("ここまで"):
            continue
        if note.endswith("後") and note[:-1] + "前" in known:
            continue
        if note in closers:
            # 他の開始注記の終了として検証される
            continue
        close = close_of(note, notes)
        body = f"［＃{note}］テスト本文" + (f"［＃{close}］" if close else "")
        if close is None:
            unclosed.append(note)
        rows.append((f"tag-{len(rows):04}", note, body))
    print(f"  (対になる終了注記なし: {len(unclosed)})", flush=True)
    return rows


def suffix_rows():
    """chuki_tag_suf.txt の (見出し, 注記テキスト) 一覧。"""
    rows = []
    for line in suffix_table_text().splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        fields = line.split("\t")
        suffix = fields[0].strip()
        if suffix:
            # 実際の用法どおり対象文字列の後ろに置く
            rows.append(
                (f"suf-{len(rows):04}", f"「テスト対象本文」{suffix}",
                 f"テスト対象本文［＃「テスト対象本文」{suffix}］")
            )
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

    sync_tables()
    rows = tag_rows() + suffix_rows()
    if args.only:
        rows = [r for r in rows if args.only in r[2]]
    if args.limit:
        rows = rows[: args.limit]
    print(f"notes: {len(rows)}", flush=True)

    shutil.rmtree(work, ignore_errors=True)
    src = work / "src"
    src.mkdir(parents=True)
    paths = []
    for key, _note, body in rows:
        path = src / f"{key}.txt"
        path.write_text(f"表題{key}\n著者{key}\n\n{body}\n", encoding="cp932")
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

    by_key = {key: note for key, note, _ in rows}
    tag_tags = {}
    for line in tag_table_text().splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        fields = line.split("\t")
        note = fields[0].strip()
        if note:
            tag_tags[note] = [f.strip() for f in fields[1:]
                              if f.strip() and f.strip() not in ("1", "2", "3", "P", "M", "L", "K")]
    # suffix 注記 (chuki_tag_suf.txt) の期待タグは、その開始/終了注記名に対応する
    # chuki_tag.txt のタグ列。suf 行も gap 判定の対象にする。
    for line in suffix_table_text().splitlines():
        fields = line.split("\t")
        if len(fields) < 3 or line.startswith("#") or not line.strip():
            continue
        suffix, start_note, end_note = (f.strip() for f in fields[:3])
        if not suffix or not start_note:
            continue
        expected = list(tag_tags.get(start_note, []))
        expected.extend(tag_tags.get(end_note, []))
        if expected:
            tag_tags[f"「テスト対象本文」{suffix}"] = expected

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
