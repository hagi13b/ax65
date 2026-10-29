;;; ax65-mode.el --- Major mode for AX65 1.0 -*- lexical-binding: t -*-

;;; Commentary:
;; AX65 言語（.ax65 / .target / ax65.proj）用のメジャーモード。

;;; Code:

(defgroup ax65 nil "AX65 language." :group 'languages)

(defcustom ax65-indent-offset 4
  "AX65 のインデント幅。" :type 'integer :group 'ax65)

(defconst ax65--keywords '("if" "else" "match" "loop" "while" "for" "in" "break" "continue" "return" "as" "const" "enum" "extern" "fn" "impl" "let" "pub" "static" "struct" "type" "unsafe" "use" "range" "flags" "fnset" "interface" "io" "shared" "init" "comptime" "asm" "atomic" "cycles" "io_span" "max_cycles" "target" "extends" "region" "reserve" "place" "encoding" "bind" "project" "never" "halt"))
(defconst ax65--reserved '("mut"))
(defconst ax65--types '("u8" "i8" "u16" "i16" "u24" "i24" "u32" "i32" "bool" "char" "str" "zstr" "histr" "bcd8" "bcd16" "bcd32" "ptr" "strbuf" "int" "real" "Option"))
(defconst ax65--constants '("true" "false" "Some" "None"))
(defconst ax65--builtins '("irq_enable" "irq_disable" "wait_interrupt" "delay" "size_of" "include_bin" "target" "build" "Cpu" "sin" "cos" "sqrt" "round" "floor" "abs" "PI" "core"))
(defconst ax65--mnemonics '("adc" "and" "asl" "bbr0" "bbr1" "bbr2" "bbr3" "bbr4" "bbr5" "bbr6" "bbr7" "bbs0" "bbs1" "bbs2" "bbs3" "bbs4" "bbs5" "bbs6" "bbs7" "bcc" "bcs" "beq" "bit" "bmi" "bne" "bpl" "bra" "brk" "bvc" "bvs" "clc" "cld" "cli" "clv" "cmp" "cpx" "cpy" "dec" "dex" "dey" "eor" "inc" "inx" "iny" "jmp" "jsr" "lda" "ldx" "ldy" "lsr" "nop" "ora" "pha" "php" "phx" "phy" "pla" "plp" "plx" "ply" "rmb0" "rmb1" "rmb2" "rmb3" "rmb4" "rmb5" "rmb6" "rmb7" "rol" "ror" "rti" "rts" "sbc" "sec" "sed" "sei" "smb0" "smb1" "smb2" "smb3" "smb4" "smb5" "smb6" "smb7" "sta" "stp" "stx" "sty" "stz" "tax" "tay" "trb" "tsb" "tsx" "txa" "txs" "tya" "wai" "smc"))

(defconst ax65-font-lock-keywords
  `(("#\\[[^]]*\\]" . font-lock-preprocessor-face)
    ("\"\\(@[[:alpha:]_][[:alnum:]_]*\\)" 1 font-lock-constant-face)
    ("'[[:alpha:]_][[:alnum:]_]*\\_>\\('\\)?"
     (0 (unless (match-beginning 1) font-lock-constant-face)))
    (,(regexp-opt ax65--reserved 'symbols) . font-lock-warning-face)
    (,(regexp-opt ax65--keywords 'symbols) . font-lock-keyword-face)
    (,(regexp-opt ax65--types 'symbols) . font-lock-type-face)
    (,(regexp-opt ax65--constants 'symbols) . font-lock-constant-face)
    (,(regexp-opt ax65--builtins 'symbols) . font-lock-builtin-face)
    ("\\_<[A-Z][A-Z0-9_]+\\_>" . font-lock-constant-face)
    ("\\_<[A-Z][a-z0-9][A-Za-z0-9_]*\\_>" . font-lock-type-face)
    ("\\(?:\\$[0-9A-Fa-f_]+\\|\\_<0x[0-9A-Fa-f_]+\\|\\_<0b[01_]+\\|\\_<[0-9][0-9_]*\\)" . font-lock-constant-face)
    (ax65--match-mnemonic . font-lock-function-name-face)
    ("{[[:alpha:]_][[:alnum:]_.:]*}" . font-lock-variable-name-face))
  "AX65 のハイライト規則。")

(defvar ax65-mode-syntax-table
  (let ((st (make-syntax-table)))
    (modify-syntax-entry ?/ ". 124b" st)
    (modify-syntax-entry ?* ". 23" st)
    (modify-syntax-entry ?\n "> b" st)
    (modify-syntax-entry ?\" "\"" st)
    (modify-syntax-entry ?\' "." st)
    (modify-syntax-entry ?_ "_" st)
    (modify-syntax-entry ?$ "_" st)
    st)
  "AX65 の構文表。")

(defun ax65--in-asm-p (&optional pos)
  "POS（省略時はポイント）が asm ブロック内なら non-nil。"
  (save-excursion
    (let ((ppss (syntax-ppss pos)) found)
      (dolist (open (nth 9 ppss))
        (goto-char open)
        (when (looking-back "\\_<asm\\_>[^{\n]*" (line-beginning-position))
          (setq found t)))
      found)))

(defun ax65--match-mnemonic (limit)
  "asm ブロック内の 6502 ニーモニックを探す。"
  (let ((re (concat "^[ \t]*\\(?:[@A-Za-z_][A-Za-z0-9_]*:[ \t]*\\)?\\("
                    (regexp-opt ax65--mnemonics) "\\)\\_>"))
        (case-fold-search t) found)
    (while (and (not found) (re-search-forward re limit t))
      (when (ax65--in-asm-p (match-beginning 1))
        (setq found t)
        (set-match-data (list (match-beginning 1) (match-end 1)))))
    found))

(defun ax65--asm-comment-propertize (start end)
  "asm ブロック内の `;' 以降をコメント扱いにする。"
  (goto-char start)
  (while (re-search-forward ";" end t)
    (let ((pos (match-beginning 0)))
      (when (and (not (nth 8 (save-excursion (syntax-ppss pos))))
                 (ax65--in-asm-p pos))
        (put-text-property pos (1+ pos) 'syntax-table (string-to-syntax "<"))))))

(defun ax65-indent-line ()
  "括弧の深さでインデントする。"
  (interactive)
  (let* ((depth (save-excursion (beginning-of-line) (car (syntax-ppss))))
         (closing (save-excursion (back-to-indentation) (looking-at "[]})]")))
         (col (* ax65-indent-offset (max 0 (if closing (1- depth) depth)))))
    (if (<= (current-column) (current-indentation))
        (indent-line-to col)
      (save-excursion (indent-line-to col)))))

;;;###autoload
(define-derived-mode ax65-mode prog-mode "AX65"
  "AX65 言語用のメジャーモード。"
  :syntax-table ax65-mode-syntax-table
  (setq-local comment-start "// ")
  (setq-local comment-end "")
  (setq-local syntax-propertize-function #'ax65--asm-comment-propertize)
  (setq-local font-lock-defaults '(ax65-font-lock-keywords))
  (setq-local indent-line-function #'ax65-indent-line))

;;;###autoload
(add-to-list 'auto-mode-alist '("\\.\\(?:ax65\\|target\\)\\'" . ax65-mode))
;;;###autoload
(add-to-list 'auto-mode-alist '("/ax65\\.proj\\'" . ax65-mode))

(provide 'ax65-mode)
;;; ax65-mode.el ends here
