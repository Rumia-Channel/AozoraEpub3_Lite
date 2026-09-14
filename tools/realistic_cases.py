"""実際の青空文庫テキストに近い形で Java / Rust の出力を比較する。

注記表の網羅テスト (`note_coverage.py`) は「注記＋本文を同一行に置く」という
非現実的な形で差を拾うため、ここでは現実的な用法だけを並べる。

Narou.rb / Narou Bridge のカスタム注記は `narou.rs` が所有する
(`preset/custom_chuki_tag.txt`、インストール先への書き込みは narou の init)。
Lite 本体の資産には含まれないため、Narou グループだけは

  Java : 配布物ディレクトリを CWD にして実行 (narou init 済みの表)
  Rust : `--config-dir` に narou のプリセットを `custom_chuki_tag.txt` として渡す

という、実際の narou → Lite の経路で比較する。
"""

import os
import pathlib
import re
import shutil
import zipfile

from java_reference import (
    CLASSES,
    DIST,
    JAR,
    JAVABIN,
    RUST,
    RUSTBIN,
    run,
    sync_tables,
    sync_templates,
)

WORK = RUST / "target" / "realistic"
NAROU_PRESET = pathlib.Path(
    os.environ.get(
        "NAROU_PRESET",
        "C:/Users/rumia/Desktop/APP/Rust/narou.rs/preset/custom_chuki_tag.txt",
    )
)

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
}

NAROU_CASES = {
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
    for name in sorted(
        n for n in epub if n.startswith("item/xhtml/") and n.endswith(".xhtml")
    ):
        text = epub[name]
        start = text.find("<body")
        if start < 0:
            continue
        start = text.find(">", start) + 1
        parts.append(text[start : text.rfind("</body>")])
    return "".join(parts).strip()


def compare(
    cases: dict[str, str],
    java_cwd: pathlib.Path,
    rust_args: list[str],
    label: str,
) -> tuple[int, int]:
    java_out = WORK / f"j-{label}"
    rust_out = WORK / f"r-{label}"
    shutil.rmtree(java_out, ignore_errors=True)
    shutil.rmtree(rust_out, ignore_errors=True)
    java_out.mkdir(parents=True)
    rust_out.mkdir(parents=True)

    paths = []
    for key, text in cases.items():
        path = WORK / "src" / f"{key}.txt"
        path.write_text(f"表題{key}\n著者{key}\n\n{text}", encoding="cp932", newline="")
        paths.append(str(path))

    code, log = run(
        ["java", "-cp", f"{CLASSES};{JAR}", "AozoraEpub3", "-ext", ".epub",
         "-d", str(java_out), *paths],
        java_cwd,
    )
    if code != 0:
        print(f"java exit {code}: {log[:300]}")
    code, log = run(
        [str(RUSTBIN), *rust_args, "-ext", ".epub", "-d", str(rust_out), *paths], RUST
    )
    if code != 0:
        print(f"rust exit {code}: {log[:300]}")

    same = 0
    for key in cases:
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
    print(f"[{label}] 一致 {same}/{len(cases)}")
    return same, len(cases)


def main() -> None:
    sync_tables()
    sync_templates()
    shutil.rmtree(WORK, ignore_errors=True)
    (WORK / "src").mkdir(parents=True)

    compare(CASES, JAVABIN, [], "aozora")

    if NAROU_PRESET.is_file():
        # Java 側は「上流表 + narou のプリセット」だけを差とした作業ディレクトリで
        # 実行する (配布物ディレクトリ直下には INI があり、タイトルページ設定まで
        # 拾って比較条件がずれるため)。
        javabin_narou = WORK / "narou-javabin"
        shutil.copytree(JAVABIN, javabin_narou, dirs_exist_ok=True)
        shutil.copyfile(DIST / "chuki_tag.txt", javabin_narou / "chuki_tag.txt")
        config_dir = WORK / "narou-cfg"
        config_dir.mkdir(parents=True)
        shutil.copyfile(NAROU_PRESET, config_dir / "custom_chuki_tag.txt")
        compare(NAROU_CASES, javabin_narou, ["--config-dir", str(config_dir)], "narou")
    else:
        print(f"Narou プリセットが見つからないためスキップ: {NAROU_PRESET}")


if __name__ == "__main__":
    main()
