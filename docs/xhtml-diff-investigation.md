# 本家 AozoraEpub3 (Java) との EPUB 差分調査メモ

- 調査日: 2026-08-23
- 比較方法: EPUB 解凍し `item/xhtml/*.xhtml` + `item/style/*.css` を CR 差異を除き byte 比較
- 結果: 97 ファイル中 56 一致 / 41 不一致。不一致は下記 4 パターンのみ。
- 本家参照ソース: `sample/AozoraEpub3/src/com/github/hmdev/...`
- 検証データ: `C:\Users\rumia\Documents\WebNocel\小説データ\小説家になろう\n0449mj 牌アガる！-High!-\`
  - `[ぺー村様] 牌アガる！-High!-.epub` = 本家 Java 版出力 (47 xhtml, text.css `margin: 0 0 0 0`)
  - 同 `.txt` = 元テキスト

---

## パターンA: 節末尾に `<p><br/></p>` が 1 行増える (36 ファイル)

```diff
 （本文の最終行）
+ <p><br/></p>
 </div>
```

### 原因
- Java `printLineBuffer` (`AozoraEpub3Converter.java:3291-3298, 3370-3384`):
  - 空行は即時出力せず `printEmptyLines++` でカウント。
  - 次の非空行出力時に `lines = min(maxEmptyLine, printEmptyLines - removeEmptyLine)` 行ぶんフラッシュ。
    - デフォルト: removeEmptyLine=0, maxEmptyLine=無制限 → 連続空行は圧縮されず全件出力。
    - 見出し直前の空行は `lines = max(1, lines)` で最低 1 行保証 (`:3374-3376`)。
  - **改ページトリガ設定時 `setPageBreakTrigger()` 内で `printEmptyLines = 0` にリセット** (`:3262-3264`, コメント「改ページ前の空行は無視」) → 節末尾の空行は出力されない。
- Lite `append_line` / `append_block_line` (`src/text.rs:1591-1592, 1624-1625`):
  - 空行を `<p><br/></p>\n` **即時出力**するため、節区切り (改ページ) 直前の空行が破棄されない。

### 修正方針 (案)
- 空行の出力を遅延化 (ペンディングカウンタ) し、次の非空行出力時にまとめてフラッシュ。
- 改ページ注記処理 (`page_break_note` ルート, `text.rs:964-971`) と render_lines 終端で未フラッシュ空行を破棄。
- 注意: 閉じタグ行 (`［＃ここで○○終わり］`) の直前にある空行は Java もフラッシュする
  (`printLineBuffer` の空行フラッシュは noBr 引数に依存しないため)。破棄は改ページ時のみ。
- 未確認: 「見出し後3行以内開始の空行は1行残す」(`:3374`) の lastChapterLine 相当管理の要否。
  現状比較では見出し前後の空行は差分なしのため、遅延化しても挙動は同じはず。

---

## パターンB: U+FF0D「－」→ U+2015「―」変換 [最重要] (3 ファイル)

```diff
- <p>マイ「－－－ンゥノコッタァァァッ!!…」</p>   ← －= U+FF0D 全角マイナス
+ <p>マイ「―――ンゥノコッタァァァッ!!…」</p>   ← ―= U+2015 水平バー
```

### 原因
- `assets/aozora/replace.txt:8` にルール `－	―` があり、Lite がこれを**常時適用**している
  (`src/main.rs:80-81` コメント「bundled replace.txt rules stay active (。」→」, －→―, ＜→〈, ＞→〉)」)。
- 本家は replace.txt を GUI/設定で明示指定した場合のみ読み込む (`AozoraEpub3.java`)。
  デフォルト (= replace.txt 無指定) では一切置換せず U+FF0D を保持する。
- reader 内検索・コピー&ペーストで元文字 `－` が引っかからなくなる実害。

### 修正方針 (案)
- bundled replace.txt の自動適用を廃止。本家同様、オプション (--replace-file 相当) 指定時のみ適用。
- 既存 CLI オプション構成 (`src/cli_config.rs`) との整合を確認してから実装。
- 影響: `。」→」` `＜→〈` `＞→〉` ルールも同時に無効になる (本家デフォルト挙動としては正しい)。

---

## パターンC: 巻末ブロックが p タグで分割される (1 ファイル)

元テキスト (1 行):
```
［＃ここから地付き］［＃小書き］（本を読み終わりました）［＃小書き終わり］［＃ここで地付き終わり］
```

```diff
- <div class="btm"><span class="kogaki">（本を読み終わりました）</span></div>
+ <div class="btm">
+ <p><span class="kogaki">（本を読み終わりました）</span></p>
+ </div>
```

### 原因
- Java: 行全体を 1 バッファで注記→タグ置換し 1 行出力。
  `printLineBuffer` の isBlockTag 判定 `^\s*<(h\d|div|table|ul|ol|li|blockquote|section|article|header|footer)\b`
  (`AozoraEpub3Converter.java:3387, 3389`) により `<div` で始まる行は p ラップされない。
- Lite: `split_block_notes` (`src/text.rs:1273`) が block 注記マーカーで行を 3 片に分解し、
  `merge_same_line_block_pieces` (`src/text.rs:1315`) の結合条件のうち
  `!content.trim().starts_with("［＃")` (`:1334`) が「インライン注記 (`［＃小書き］…`) で始まる内容」も
  結合対象から排除するため、中間片が独立行として `<p>` ラップされる。

### 修正方針 (案)
- `merge_same_line_block_pieces` の content 条件を緩和:
  content が「block_open_tags / block_close_tags の注記**単独片**」である場合のみ結合禁止とし、
  インライン注記を含む内容片は結合許可にする。
  (block markers でしか分割されないため、「`［＃` 始まりだが block 注記単独ではない」片は必ずインライン内容を含む)

---

## パターンD: text.css `@page` margin ハードコード (1 ファイル)

```diff
  @page {
-   margin: 0 0 0 0;
+   margin: 0.5em 0.5em 0.5em 0.5em;   ← Lite はハードコード (epub.rs:20)
  }
```

### 原因
- 本家: `template/item/style/text.vm` が `margin: ${pageMargin[0]} ${pageMargin[1]} ${pageMargin[2]} ${pageMargin[3]};`
  `Epub3Writer.java:216` のデフォルト `{"0","0","0","0"}` → 出力 `margin: 0 0 0 0;`。
- Lite: `src/epub.rs:15-73` TEXT_CSS 定数内 `:20` で `margin: 0.5em ...;` をハードコード。

### 修正方針 (案)
- `margin: 0 0 0 0;` に変更 (本家デフォルト一致)。
- 将来的には pageMargin / bodyMargin / lineHeight / fontSize を設定化できる余地あり
  (本家はプロファイルから setStyles で注入)。本修正ではデフォルト一致のみ。

---

## 次の作業 (別 PC 引き継ぎ用チェックリスト)

1. [ ] パターンB: replace.txt 自動適用廃止 + CLI オプション整備 (`cli_config.rs` 確認)
2. [ ] パターンA: 空行遅延フラッシュ + 改ページ時破棄 (`text.rs` append_line / append_block_line / page_break_note)
3. [ ] パターンC: merge_same_line_block_pieces 条件緩和 (`text.rs:1315-1353`)
4. [ ] パターンD: epub.rs:20 margin を `0 0 0 0` へ
5. [ ] 各修正後に cargo test + 実データ (牌アガる！等) で本家 EPUB との再 byte 比較
6. [ ] 既存テストで Java 互換コメント付きのもの (`main.rs:432`, `text_tests.rs` 等) の期待値調整確認
