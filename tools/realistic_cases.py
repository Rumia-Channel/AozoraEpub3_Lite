"""実際の青空文庫テキストに近い形で Java / Rust の出力を比較する。

注記表の網羅テスト (note_coverage.py) は「注記＋本文を同一行に置く」という
非現実的な形で差を拾うため、ここでは現実的な用法だけを並べる。
"""

import pathlib
import re
import shutil
import subprocess
import zipfile

RUST = pathlib.Path(__file__).resolve().parent.parent
JAVA_REPO = pathlib.Path("C:/Users/rumia/Desktop/APP/Java/AozoraEpub3")
WORK = RUST / "target" / "realistic"
CLASSES = RUST / "target" / "audit-diff" / "classes"
JAR = pathlib.Path("C:/Users/rumia/Documents/AozoraEpub3/AozoraEpub3.jar")
JAVABIN = RUST / "target" / "audit-diff" / "javabin"
RUSTBIN = RUST / "target" / "release" / "AozoraEpub3_Lite.exe"

CASES = {
    "page-bottom": "［＃ページ左下］\nテキスト\n",
    "page-bottom-line": "テキスト\n［＃ページ左下］\n次のテキスト\n",
    "page-center": "［＃ページの左右中央］\nテキスト\n",
    "page-break": "［＃改ページ］\nテキスト\n",
    "bold-block": "［＃ここから太字］\nテキスト\n［＃ここまで太字］\n",
    "heading-close": "［＃大見出し］章名［＃大見出し終わり］\n",
    "heading-open": "［＃大見出し］章名\n",
    "indent-block": "［＃ここから３字下げ］\n本文\n［＃ここで字下げ終わり］\n",
    "indent-wrap": "［＃ここから３字下げ、折り返して２字下げ］\n本文\n［＃ここで字下げ終わり］\n",
    "indent-inline": "本文［＃３字下げ］テスト［＃字下げ終わり］\n",
    "jizume": "［＃ここから５字詰め］\n本文\n［＃ここで字詰め終わり］\n",
    "font-step-block": "［＃ここから１段階大きな文字］本文［＃ここで大きな文字終わり］\n",
    "font-step-inline": "［＃１段階大きな文字］本文［＃大きな文字終わり］\n",
    "chikuzuke": "テキスト［＃地付き］テスト本文［＃地付き終わり］\n",
    "title-wrap": "［＃表題前］書名［＃表題後］\n",
    "em-block": "［＃ここから傍点］本文［＃ここで傍点終わり］\n",
    "inline-chuki": "テキスト［＃「テキスト」に傍点］テスト本文\n",
    "indent-border-center": "［＃ここから３字下げ、罫囲みと中央揃え］\n本文\n［＃ここで字下げ終わり］\n",
    "indent-dashed-frame": "［＃ここから２字下げ、破線枠囲み］\n本文\n［＃ここで字下げ終わり］\n",
    "indent-jizume": "［＃ここから３字下げ、５字詰め］\n本文\n［＃ここで字下げ終わり］\n",
    "indent-dashed-kekomi": "［＃ここから２字下げ、破線罫囲み］\n本文\n［＃ここで字下げ終わり］\n",
    "indent-yoko": "［＃ここから２字下げ、横書き］\n本文\n［＃ここで字下げ終わり］\n",
    # 配布物 (Narou.rb / Narou Bridge 同梱) のカスタム注記
    "narou-hashira": "［＃ここから柱］\n柱の本文\n［＃ここで柱終わり］\n本文\n",
    "narou-preface": "［＃ここから前書き］\n前書き本文\n［＃ここで前書き終わり］\n本文\n",
    "narou-postscript": "［＃ここから後書き］\n後書き本文\n［＃ここで後書き終わり］\n",
    "narou-parameter": "［＃ここからパラメーター］\n名前：テスト\n［＃ここでパラメーター終わり］\n",
    "narou-indent": "［＃一字下げ］字下げ本文\n",
    "narou-nibuaki": "テキスト［＃二分アキ］テスト本文\n",
    "narou-zws": "テキスト［＃zws］テスト本文\n",
    "narou-dakuten": "テキスト［＃濁点］テスト本文［＃濁点終わり］\n",
    "narou-link": "［＃ｌｉｎｋ＿ｓ］https://example.com［＃ｌｉｎｋ＿ｔ］リンク［＃ｌｉｎｋ＿ｅ］\n",
}


def entries(path: pathlib.Path) -> dict[str, str]:
    with zipfile.ZipFile(path) as archive:
        return {
            name: archive.read(name).replace(b"\r\n", b"\n").decode("utf-8", "replace")
            for name in archive.namelist()
        }


def body_of(epub: dict[str, str]) -> str:
    """本文 XHTML (item/xhtml/*.xhtml) の <body> 中身を連結して返す。"""
    parts = []
    for name in sorted(n for n in epub
                       if n.startswith("item/xhtml/") and n.endswith(".xhtml")):
        text = epub[name]
        start = text.find("<body")
        if start < 0:
            continue
        start = text.find(">", start) + 1
        end = text.rfind("</body>")
        parts.append(text[start:end])
    return "".join(parts).strip()


def run(command: list[str], cwd: pathlib.Path) -> tuple[int, str]:
    done = subprocess.run(
        command, cwd=cwd, capture_output=True, text=True,
        encoding="utf-8", errors="replace",
    )
    return done.returncode, (done.stdout or "") + (done.stderr or "")


def main() -> None:
    shutil.rmtree(WORK, ignore_errors=True)
    source = WORK / "src"
    source.mkdir(parents=True)
    paths = []
    for key, text in CASES.items():
        path = source / f"{key}.txt"
        path.write_text(
            f"表題{key}\n著者{key}\n\n{text}", encoding="cp932", newline=""
        )
        paths.append(str(path))

    java_out = WORK / "j"
    rust_out = WORK / "r"
    java_out.mkdir()
    rust_out.mkdir()
    code, log = run(
        ["java", "-cp", f"{CLASSES};{JAR}", "AozoraEpub3", "-ext", ".epub",
         "-d", str(java_out), *paths],
        JAVABIN,
    )
    if code != 0:
        print("java exit", code, log[:400])
    code, log = run(
        [str(RUSTBIN), "-ext", ".epub", "-d", str(rust_out), *paths], RUST
    )
    if code != 0:
        print("rust exit", code, log[:400])

    same = 0
    for key in CASES:
        try:
            java = body_of(entries(next(java_out.glob(f"*{key}.epub"))))
            rust = body_of(entries(next(rust_out.glob(f"*{key}.epub"))))
        except (IndexError, KeyError, OSError) as error:
            print(f"{key}: 生成失敗 ({error})")
            continue
        if not java or not rust:
            print(f"{key}: 抽出失敗 java={len(java)} rust={len(rust)}")
            continue
        if java == rust:
            same += 1
            continue
        print(f"\n{key}: 差分")
        java_lines = java.split("<")
        rust_lines = rust.split("<")
        shown = 0
        for index in range(max(len(java_lines), len(rust_lines))):
            left = java_lines[index] if index < len(java_lines) else "(なし)"
            right = rust_lines[index] if index < len(rust_lines) else "(なし)"
            if left != right:
                print(f"  java: <{left[:120]}")
                print(f"  rust: <{right[:120]}")
                shown += 1
                if shown >= 4:
                    break
    print(f"\n一致 {same}/{len(CASES)}")


if __name__ == "__main__":
    main()
