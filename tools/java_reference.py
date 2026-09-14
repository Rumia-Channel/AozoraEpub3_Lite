"""Java 参照実装を動かすための共通パスと、参照データの同期。

監査ハーネス (note_coverage / realistic_cases / parity_check) は、
リポジトリ src を javac でビルドしたクラスを `target/audit-diff/classes` に置き、
インストール済み配布物 (`Documents/AozoraEpub3`) の jar・注記資産・テンプレートを
`target/audit-diff/javabin` に置いて、そこを CWD にして Java を実行する。

注記資産は上流リポジトリ (`JAVA_REPO`) 側を正とする。Narou.rb / Narou Bridge
のカスタム注記は narou.rs が `preset/custom_chuki_tag.txt` として所有し、
インストール先の `chuki_tag.txt` へ書き込む (`narou.rs` の `init` コマンド) もの
なので、Lite 本体の資産には含めない。配布物の表は narou の init 後であり、
比較したい場合は `DIST` を CWD にして Java を動かす (`realistic_cases.py` の
Narou グループがこの経路)。
"""

import pathlib
import shutil
import subprocess

RUST = pathlib.Path(__file__).resolve().parent.parent
JAVA_REPO = pathlib.Path("C:/Users/rumia/Desktop/APP/Java/AozoraEpub3")
DIST = pathlib.Path("C:/Users/rumia/Documents/AozoraEpub3")
CLASSES = RUST / "target" / "audit-diff" / "classes"
JAVABIN = RUST / "target" / "audit-diff" / "javabin"
JAR = DIST / "AozoraEpub3.jar"
RUSTBIN = RUST / "target" / "release" / "AozoraEpub3_Lite.exe"

TABLES = (
    "chuki_tag.txt",
    "chuki_tag_suf.txt",
    "chuki_utf.txt",
    "chuki_ivs.txt",
    "chuki_alt.txt",
    "chuki_latin.txt",
)


def sync_tables() -> None:
    """Java 実行ディレクトリの注記資産を上流リポジトリのものへ揃える。"""
    for name in TABLES:
        source = JAVA_REPO / name
        if source.is_file():
            shutil.copyfile(source, JAVABIN / name)


def tag_table_text() -> str:
    """参照側 (Java) が使う注記表。"""
    return (JAVABIN / "chuki_tag.txt").read_text("utf-8", errors="replace")


def suffix_table_text() -> str:
    return (JAVABIN / "chuki_tag_suf.txt").read_text("utf-8", errors="replace")


def run(command: list[str], cwd: pathlib.Path) -> tuple[int, str]:
    done = subprocess.run(
        command,
        cwd=cwd,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    return done.returncode, (done.stdout or "") + (done.stderr or "")
