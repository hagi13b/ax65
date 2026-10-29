// ax65 — AX65 言語のリファレンスコンパイラ（仕様書 1.0）
mod ast;
mod asm6502;
mod asmtext;
mod cli;
mod cgexpr;
mod codegen;
mod check;
mod comptime;
mod diag;
mod explain;
mod lexer;
mod opt;
mod output;
mod parser;
mod program;
mod project;
mod report;
mod sema;
mod types;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = cli::run(args);
    std::process::exit(code);
}
