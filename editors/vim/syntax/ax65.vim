" Vim syntax file
" Language: AX65 1.1
if exists("b:current_syntax")
  finish
endif
syn case match
syn keyword ax65Control if else match loop while for in break continue return
syn keyword ax65Keyword as const enum extern fn impl let pub static struct type unsafe use range flags fnset interface io shared init comptime atomic cycles io_span max_cycles
syn match ax65Keyword "\<asm\>"
syn keyword ax65Target target extends region reserve place encoding bind project never halt
syn keyword ax65Reserved mut
syn keyword ax65Type u8 i8 u16 i16 u24 i24 u32 i32 bool char str zstr histr bcd8 bcd16 bcd32 ptr strbuf int real Option
syn keyword ax65Constant true false Some None
syn keyword ax65Builtin irq_enable irq_disable wait_interrupt delay size_of include_bin build Cpu sin cos sqrt round floor abs PI core
syn match ax65UserType "\<[A-Z][a-z0-9][A-Za-z0-9_]*\>"
syn match ax65ConstName "\<[A-Z][A-Z0-9_]\+\>"
syn match ax65FuncCall "\<[a-z_][A-Za-z0-9_]*\ze\s*("
syn match ax65FuncName "\%(\<fn\s\+\)\@<=\h\w*"
syn match ax65Number "\$[0-9A-Fa-f_]\+"
syn match ax65Number "\<0x[0-9A-Fa-f_]\+\>"
syn match ax65Number "\<0b[01_]\+\>"
syn match ax65Number "\<[0-9][0-9_]*\(\.[0-9]\+\)\=\>"
syn match ax65Escape contained "\\\(x[0-9A-Fa-f]\{2}\|[nrt0\\'\"]\)"
syn region ax65String start=+\%(\<\%(p\|z\|hi\)\)\="+ skip=+\\\\\|\\"+ end=+"\(@\h\w*\)\=+ contains=ax65Escape
syn match ax65Char "'\(\\\(x[0-9A-Fa-f]\{2}\|.\)\|[^'\\]\)'\(@\h\w*\)\=" contains=ax65Escape
syn match ax65Label "'\h\w*\>'\@!"
syn match ax65Encoding "\w\@<=@\h\w*"
syn match ax65AttrName contained "\<\h\w*\>"
syn region ax65Attribute start="#\[" end="\]" contains=ax65Number,ax65String,ax65AttrName
syn keyword ax65Todo contained TODO FIXME XXX NOTE
syn match ax65LineComment "//.*$" contains=ax65Todo
syn region ax65BlockComment start="/\*" end="\*/" contains=ax65Todo
" インラインアセンブリ: asm { ... }、asm (...) { ... }、asm fn ...(...) { ... }、unsafe asm (...) { ... }
" asm から { までの見出しは通常のコード、{ から } までは 65C02 のアセンブリとして色付けする。
" {名前} の参照は ax65AsmRef が先に取り込むので、ブロックの終わりの } と混ざらない
syn case ignore
syn keyword ax65Mnemonic contained adc and asl bbr0 bbr1 bbr2 bbr3 bbr4 bbr5 bbr6 bbr7 bbs0 bbs1 bbs2 bbs3 bbs4 bbs5 bbs6 bbs7 bcc bcs beq bit bmi bne bpl bra brk bvc bvs clc cld cli clv cmp cpx cpy dec dex dey eor inc inx iny jmp jsr lda ldx ldy lsr nop ora pha php phx phy pla plp plx ply rmb0 rmb1 rmb2 rmb3 rmb4 rmb5 rmb6 rmb7 rol ror rti rts sbc sec sed sei smb0 smb1 smb2 smb3 smb4 smb5 smb6 smb7 sta stp stx sty stz tax tay trb tsb tsx txa txs tya wai smc
syn keyword ax65AsmRegister contained a x y
syn case match
syn match ax65AsmComment contained ";.*$"
syn match ax65AsmRef contained "{[^{}]*}"
syn match ax65AsmLabel contained "^\s*\zs[@A-Za-z_][A-Za-z0-9_]*:"
syn match ax65AsmLabel contained "\%(\<smc\s\+\)\@<=[@A-Za-z_][A-Za-z0-9_]*:"
syn match ax65AsmNumber contained "#\=\$[0-9A-Fa-f_]\+\|#\=\<0x[0-9A-Fa-f_]\+\>\|#\=\<0b[01_]\+\>\|#\=\<[0-9][0-9_]*\>"
syn region ax65AsmBody contained matchgroup=ax65AsmBrace start="{" end="}" contains=ax65Mnemonic,ax65AsmRegister,ax65AsmComment,ax65LineComment,ax65BlockComment,ax65AsmRef,ax65AsmLabel,ax65AsmNumber,ax65Char
syn region ax65Asm matchgroup=ax65Keyword start="\<asm\>\ze[^{};]*{" end="\%(}\)\@<=" contains=ax65AsmBody,ax65LineComment,ax65BlockComment,ax65Control,ax65Keyword,ax65Type,ax65UserType,ax65ConstName,ax65FuncName,ax65FuncCall,ax65Number
hi def link ax65Control Conditional
hi def link ax65Keyword Keyword
hi def link ax65Target Structure
hi def link ax65Reserved Error
hi def link ax65Type Type
hi def link ax65UserType Type
hi def link ax65ConstName Constant
hi def link ax65Constant Boolean
hi def link ax65Builtin Function
hi def link ax65FuncName Function
hi def link ax65FuncCall Function
hi def link ax65Number Number
hi def link ax65String String
hi def link ax65Char Character
hi def link ax65Escape SpecialChar
hi def link ax65Label Label
hi def link ax65Encoding Special
hi def link ax65Attribute PreProc
hi def link ax65AttrName PreProc
hi def link ax65Todo Todo
hi def link ax65LineComment Comment
hi def link ax65BlockComment Comment
hi def link ax65Mnemonic Statement
hi def link ax65AsmRegister Identifier
hi def link ax65AsmComment Comment
hi def link ax65AsmRef Identifier
hi def link ax65AsmLabel Label
hi def link ax65AsmNumber Number
hi def link ax65AsmBrace Delimiter
let b:current_syntax = "ax65"
