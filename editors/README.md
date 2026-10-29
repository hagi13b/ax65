# AX65 シンタックスハイライト 1.1

対象ファイル：`*.ax65`、`*.target`、`ax65.proj`。キーワードは仕様書 1.1 の 2.4節、組み込みの名前は 20.1節に合わせています。

| 色分けするもの | 例 |
|---|---|
| キーワード（制御・宣言・ブロック・ターゲット定義） | `if` `loop` `fn` `static` `range` `shared` `asm` `cycles` `target` `bind` |
| 予約語（使うとエラー） | `mut`（エラー色） |
| 型 | `u8` `i16` `bcd16` `bool` `char` `str` `zstr` `histr` `ptr` `strbuf` `Option`、大文字始まりの型名 |
| 定数・組み込み | `true` `false` `Some` `None`、大文字だけの名前、`irq_enable` `delay` `size_of` `sin` `PI` |
| 数値 | `$FF` `0xFF` `0b1010_0101` `1_000` |
| 文字列・文字 | `"…"` `p"…"` `z"…"` `hi"…"`、`'A'` `'\x41'`、エスケープ、`@screen` などの文字コード指定 |
| 属性 | `#[zp]` `#[interrupt(irq)]` |
| ラベル | `'outer` |
| インラインアセンブリ | `asm { … }` の中の 65C02 命令、`;` コメント、`{name}` 参照、ラベル |

## Visual Studio Code

`vscode/ax65` フォルダを、拡張機能フォルダへそのままコピーし、VS Code を再起動します。

- Windows：`%USERPROFILE%\.vscode\extensions\ax65`
- Linux / macOS：`~/.vscode/extensions/ax65`

`.vsix` にまとめる場合は、`vscode/ax65` で `npx @vscode/vsce package` を実行します。

## Vim / Neovim

```
cp -r vim/syntax vim/ftdetect ~/.vim/          # Neovim は ~/.config/nvim/
```

Windows の gVim は `%USERPROFILE%\vimfiles\` へコピーします。`~/.vimrc` に `syntax on` と `filetype on` が必要です。

## Emacs

```elisp
(add-to-list 'load-path "~/.emacs.d/lisp/")   ; ax65-mode.el を置いた場所
(require 'ax65-mode)
```

インデント幅は `ax65-indent-offset`（既定 4）で変えられます。

## Notepad++

メニュー「言語 → ユーザー定義言語 → ユーザー定義言語を定義…」で「インポート…」を押し、`notepadpp/AX65.xml` を選んで Notepad++ を再起動します。

## Mousepad（GtkSourceView）

```
mkdir -p ~/.local/share/gtksourceview-4/language-specs
cp mousepad/ax65.lang ~/.local/share/gtksourceview-4/language-specs/
```

Mousepad が GtkSourceView 3 を使う版なら `gtksourceview-3.0`、5 なら `gtksourceview-5` のフォルダにコピーします。同じファイルは gedit などでも使えます。

## 制限

- Notepad++ は文脈を区別できないため、65C02 の命令名（`lda`、`bit` など）を `asm` ブロックの外でも色付けします。また拡張子 `proj` を持つほかのファイルも AX65 として扱います。
- VS Code と Vim は、`asm { … }`、`asm (…) { … }`、`asm fn …(…) { … }`、`unsafe asm (…) { … }` を、`{` の後ろの `//` コメントや 1 行で書いた形も含めて認識します。ただし `asm` と `{` は同じ行に書く必要があります。
- そのほかのエディタでは、`asm` ブロックは、`asm … {` で行が終わり、`}` が行頭（空白のあと）に来る形を認識します。1 行で書いた `asm { … }` は、通常のコードとして色付けします。
