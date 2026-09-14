"""Java `JisConverter.java` から面区点テーブルを抽出して `src/jis.rs` を生成する。

使い方 (リポジトリ直下で):
    python tools/gen_jis.py
参照する Java 実装のパスは AOZORA_JAVA 環境変数で上書きできる。
"""

import pickle
import pathlib
import re

import os

JAVA = pathlib.Path(
    os.environ.get(
        "AOZORA_JAVA",
        "C:/Users/rumia/Desktop/APP/Java/AozoraEpub3/src/com/github/hmdev/converter/JisConverter.java",
    )
)
OUT = pathlib.Path("src/jis.rs")

src = JAVA.read_text(encoding="utf-8")


def method_body(name):
    start = src.index("private int[][] " + name + "()")
    end = src.index("\n\t}", start)
    return src[start:end]


def int_rows(name):
    body = method_body(name)
    return [[int(v, 16) for v in m.group(1).split(",")]
            for m in re.finditer(r"\{(0(?:,0x[0-9a-fA-F]+|,0)*)\}", body)]


def men2_rows():
    body = method_body("init2")
    rows = {}
    for m in re.finditer(r"arr\[(\d+)\]\s*=\s*new int\[\]\{(0(?:,0x[0-9a-fA-F]+|,0)*)\}", body):
        rows[int(m.group(1))] = [int(v, 16) for v in m.group(2).split(",")]
    return rows


def men0():
    start = src.index("men0 = new char[]")
    end = src.index("};", start)
    body = src[start:end]
    return re.findall(r"'(?:\\'|\\.|[^'])'", body)


def men1_13():
    m = re.search(r"men1_13 = new String\[\]\[\]\{(.*?)\n\t\t\};", src, re.S)
    body = m.group(1)
    rows = []
    for line in re.finditer(r"\{([^{}]*)\}", body):
        cells = []
        for cell in re.finditer(r"null|\"((?:\\.|[^\"\\])*)\"", line.group(1)):
            cells.append(None if cell.group(0) == "null" else cell.group(1))
        rows.append(cells)
    return rows


def java_char_escape(token):
    """Java の char リテラル 1 個を Rust の文字列に変換する。"""
    assert token.startswith("'") and token.endswith("'"), token
    inner = token[1:-1]
    if inner == "\\'":
        return "'"
    if inner == "\\\\":
        return "\\"
    if inner.startswith("\\"):
        raise ValueError("unsupported escape: " + token)
    return inner


def rust_str(value):
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


def emit():
    m0 = [java_char_escape(c) for c in men0()]
    rows13 = men1_13()
    rows1 = int_rows("init1")
    rows2 = men2_rows()
    print("men0", len(m0), "men1_13", len(rows13), "men1", len(rows1), "men2", len(rows2))
    
    out = []
    out.append("//! Java `JisConverter` の面区点テーブルを移植したもの。")
    out.append("//! `tools/gen_jis.py` で JisConverter.java から生成する。")
    out.append("")
    out.append("/// 0 面 (ASCII 相当)。")
    out.append("static MEN0: [&str; %d] = [" % len(m0))
    out.append("    " + ", ".join(rust_str(c) for c in m0))
    out.append("];")
    out.append("")
    out.append("/// 1 面 1〜13 区。複数文字になる合成文字を含むため文字列で持つ。")
    out.append("static MEN1_13: [&[&str]; %d] = [" % (len(rows13) + 1))
    out.append("    &[],")
    for row in rows13:
        cells = ["\"\"" if cell is None else rust_str(cell) for cell in row]
        out.append("    &[" + ", ".join(cells) + "],")
    out.append("];")
    out.append("")

    def emit_rows(name, rows, doc):
        out.append(doc)
        out.append("static %s: [&[u32]; %d] = [" % (name, len(rows)))
        for row in rows:
            out.append("    &[" + ", ".join("0x%05x" % v for v in row) + "],")
        out.append("];")
        out.append("")

    # Java の配列は 0〜13 区が null なので、区番号で引けるように空行を詰める
    emit_rows("MEN1", [[]] * 14 + rows1,
              "/// 1 面 14〜94 区。値は Unicode コードポイント。")
    men2_full = []
    for ku in range(1, 95):
        men2_full.append(rows2.get(ku, []))
    emit_rows("MEN2", [None] + men2_full if False else [[]] + men2_full,
              "/// 2 面 (第 4 水準)。値は Unicode コードポイント。")

    out.append("/// Java `JisConverter.toCharString`: 面区点を文字列にする。")
    out.append("pub(crate) fn to_char_string(men: u32, ku: u32, ten: u32) -> Option<String> {")
    out.append("    match men {")
    out.append("        0 => MEN0.get(ten as usize).map(|value| (*value).to_owned()),")
    out.append("        1 => {")
    out.append("            if ku == 0 || ku >= 95 || ten == 0 {")
    out.append("                return None;")
    out.append("            }")
    out.append("            if ku <= 13 {")
    out.append("                return MEN1_13[ku as usize]")
    out.append("                    .get(ten as usize)")
    out.append("                    .filter(|value| !value.is_empty())")
    out.append("                    .map(|value| (*value).to_owned());")
    out.append("            }")
    out.append("            code_to_char_string(*MEN1[ku as usize].get(ten as usize)?)")
    out.append("        }")
    out.append("        2 => {")
    out.append("            if ku == 0 || ku >= 95 || ten == 0 {")
    out.append("                return None;")
    out.append("            }")
    out.append("            code_to_char_string(*MEN2[ku as usize].get(ten as usize)?)")
    out.append("        }")
    out.append("        _ => None,")
    out.append("    }")
    out.append("}")
    out.append("")
    out.append("/// Java `codeToCharString`: 0 は無効。Java は BMP 外をサロゲートペア 2 文字で")
    out.append("/// 返すが、Rust の `char` は補助面をそのまま表せるため 1 文字で返す。")
    out.append("fn code_to_char_string(code: u32) -> Option<String> {")
    out.append("    if code == 0 {")
    out.append("        return None;")
    out.append("    }")
    out.append("    char::from_u32(code).map(String::from)")
    out.append("}")
    out.append("")
    OUT.write_text("\n".join(out), encoding="utf-8", newline="")
    print("wrote", OUT, len("\n".join(out)), "bytes")


emit()
