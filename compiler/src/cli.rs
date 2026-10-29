// コマンドライン（17.4節・17.5節）
use crate::codegen::{generate, GenOpts};
use crate::diag::{json_escape, lint_level_from, Diags, LintLevel, SourceMap, LINT_NAMES};
use crate::lexer::{tokenize_all, IntStyle, Tok};
use crate::opt::link;
use crate::output::*;
use crate::program::{find_std_dir, known_interfaces, load_program, CfgVal, OptLevel};
use crate::project::{load_project, load_targets, Project};
use crate::sema::build_ctx;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const VERSION: &str = "ax65 1.1.0（AX65 言語仕様 1.1 のリファレンスコンパイラ）";

#[derive(Clone, Default)]
struct Opts {
    cmd: String,
    positional: Vec<String>,
    project: Option<String>,
    target: Option<String>,
    all_targets: bool,
    all_levels: bool,
    opt: Option<OptLevel>,
    defines: Vec<(String, Option<String>)>,
    binds: Vec<(String, String)>,
    emit: Option<Vec<String>>,
    out_dir: Option<String>,
    deny_warnings: bool,
    lints: Vec<(String, LintLevel)>,
    json: bool,
    no_report: bool,
    lst_plain: bool,
    woz_line: usize,
    woz_run: bool,
    step_limit: Option<u64>,
    fmt_check: bool,
    verbose: bool,
}

fn usage() -> String {
    let s = "\
使い方：ax65 <コマンド> [オプション]

コマンド：
  build              コンパイルして出力する
  check              検査だけを行う（--all-targets、--all-levels で範囲を広げる）
  fmt [--check]      ソースの書き方をそろえる（16進の書き方だけ。--check はそろっているか調べるだけ）
  explain <番号>     エラー・警告の詳しい説明を表示する（例：ax65 explain E0412）
  new <名前>         プロジェクトのひな形を作る
  targets            ターゲット定義の一覧と継承・bind を表示する

オプション：
  -t, --target <名前>        ターゲットを指定する
  --all-targets              すべてのターゲットで検査する（check）
  --all-levels               すべての最適化の段階で検査する（check）
  -O0 / -O1 / -O2 / -Os      最適化の段階（既定は -O2。-Os は速さ優先）
  -D <名前>[=<値>]           build.名前 の値を決める
  --bind <インターフェース>=<モジュール>   bind を一時的に差し替える
  --emit <形式>,...          出力形式（bin woz hex rom ptp sym lst report）
  -o, --out-dir <フォルダ>   出力先
  --project <ファイル>       プロジェクトファイル（既定は ./ax65.proj）
  --deny-warnings            すべての警告をエラーにする
  -A / -W / --deny <名前>    警告ごとの扱い（allow / warn / deny）
  --json                     メッセージを JSON で出力する
  --no-report                レポートを出力しない
  --lst-plain                リスティングの命令の説明を省く
  --woz-line <n>             woz 形式（HEXMON・Wozmon 向けの 16進ダンプ）の 1 行のバイト数（既定 8）
  --woz-run                  woz 形式の最後に Wozmon の実行の命令（200R など）を付ける
  --step-limit <n>           コンパイル時の計算の手順数の上限
  -v, --verbose              経過を表示する
  --version, --help
";
    s.to_string()
}

fn parse_args(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts { woz_line: 8, ..Default::default() };
    let mut i = 0;
    let need = |i: usize, name: &str| -> Result<String, String> {
        match args.get(i + 1) {
            Some(v) => Ok(v.clone()),
            None => Err(format!("{} の後ろに値が必要です", name)),
        }
    };
    while i < args.len() {
        let a = args[i].clone();
        match a.as_str() {
            "-t" | "--target" => {
                o.target = Some(need(i, &a)?);
                i += 1;
            }
            "--all-targets" => o.all_targets = true,
            "--all-levels" => o.all_levels = true,
            "-O0" => o.opt = Some(OptLevel::O0),
            "-O1" => o.opt = Some(OptLevel::O1),
            "-O2" => o.opt = Some(OptLevel::O2),
            "-Os" => o.opt = Some(OptLevel::Os),
            "-D" => {
                let v = need(i, &a)?;
                i += 1;
                o.defines.push(split_define(&v));
            }
            "--bind" => {
                let v = need(i, &a)?;
                i += 1;
                match v.split_once('=') {
                    Some((x, y)) => o.binds.push((x.trim().to_string(), y.trim().to_string())),
                    None => return Err("--bind は インターフェース=モジュール の形で書きます".to_string()),
                }
            }
            "--emit" => {
                let v = need(i, &a)?;
                i += 1;
                let list: Vec<String> = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                for f in &list {
                    if !["bin", "woz", "hex", "rom", "ptp", "sym", "lst", "report"].contains(&f.as_str()) {
                        return Err(format!("出力形式 {} はありません", f));
                    }
                }
                o.emit = Some(list);
            }
            "-o" | "--out-dir" => {
                o.out_dir = Some(need(i, &a)?);
                i += 1;
            }
            "--project" => {
                o.project = Some(need(i, &a)?);
                i += 1;
            }
            "--deny-warnings" => o.deny_warnings = true,
            "-A" | "-W" | "--deny" => {
                let v = need(i, &a)?;
                i += 1;
                if !LINT_NAMES.contains(&v.as_str()) {
                    return Err(format!("知らない警告の名前 {} です", v));
                }
                let l = match a.as_str() {
                    "-A" => LintLevel::Allow,
                    "-W" => LintLevel::Warn,
                    _ => LintLevel::Deny,
                };
                o.lints.push((v, l));
            }
            "--json" => o.json = true,
            "--no-report" => o.no_report = true,
            "--lst-plain" => o.lst_plain = true,
            "--woz-line" => {
                let v = need(i, &a)?;
                i += 1;
                o.woz_line = v.parse().map_err(|_| "--woz-line には数を書きます".to_string())?;
            }
            "--woz-run" => o.woz_run = true,
            "--step-limit" => {
                let v = need(i, &a)?;
                i += 1;
                o.step_limit = Some(v.replace('_', "").parse().map_err(|_| "--step-limit には数を書きます".to_string())?);
            }
            "--check" => o.fmt_check = true,
            "-v" | "--verbose" => o.verbose = true,
            _ => {
                if let Some(rest) = a.strip_prefix("-D") {
                    if !rest.is_empty() {
                        o.defines.push(split_define(rest));
                        i += 1;
                        continue;
                    }
                }
                if a.starts_with('-') {
                    return Err(format!("知らないオプション {} です（ax65 --help を見てください）", a));
                }
                if o.cmd.is_empty() {
                    o.cmd = a.clone();
                } else {
                    o.positional.push(a.clone());
                }
            }
        }
        i += 1;
    }
    Ok(o)
}

fn split_define(v: &str) -> (String, Option<String>) {
    match v.split_once('=') {
        Some((a, b)) => (a.trim().to_string(), Some(b.trim().to_string())),
        None => (v.trim().to_string(), None),
    }
}

pub fn run(args: Vec<String>) -> i32 {
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("{}", VERSION);
        return 0;
    }
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{}", usage());
        return if args.is_empty() { 2 } else { 0 };
    }
    let o = match parse_args(&args) {
        Ok(o) => o,
        Err(m) => {
            eprintln!("エラー：{}", m);
            return 2;
        }
    };
    match o.cmd.as_str() {
        "build" => cmd_build(&o, true),
        "check" => cmd_build(&o, false),
        "explain" => cmd_explain(&o),
        "new" => cmd_new(&o),
        "targets" => cmd_targets(&o),
        "fmt" => cmd_fmt(&o),
        "" => {
            print!("{}", usage());
            2
        }
        c => {
            eprintln!("エラー：知らないコマンド {} です（ax65 --help を見てください）", c);
            2
        }
    }
}

fn project_path(o: &Opts) -> PathBuf {
    match &o.project {
        Some(p) => {
            let pb = PathBuf::from(p);
            if pb.is_dir() {
                pb.join("ax65.proj")
            } else {
                pb
            }
        }
        None => PathBuf::from("ax65.proj"),
    }
}

fn print_diags(o: &Opts, diags: &Diags, sm: &SourceMap) {
    if o.json {
        return;
    }
    let s = diags.render(sm);
    if !s.is_empty() {
        eprint!("{}", s);
    }
}

fn fail(o: &Opts, d: &Diags, sm: &SourceMap, t: &str) -> Result1 {
    print_diags(o, d, sm);
    if !o.json {
        eprintln!("ax65：{} はエラー {} 件、警告 {} 件で失敗しました", t, d.error_count(), d.warning_count());
    }
    Result1 { ok: false, json: format!("{{\"target\":\"{}\",\"ok\":false,\"diagnostics\":{}}}", json_escape(t), d.to_json(sm)) }
}

struct Result1 {
    ok: bool,
    json: String,
}

fn cmd_build(o: &Opts, output: bool) -> i32 {
    // 対象のターゲット
    let mut targets: Vec<Option<String>> = vec![o.target.clone()];
    if o.all_targets {
        let mut sm = SourceMap::new();
        let mut d = Diags::new();
        let p = match load_project(&project_path(o), &mut sm, &mut d) {
            Some(p) => p,
            None => {
                print_diags(o, &d, &sm);
                return 1;
            }
        };
        let ts = load_targets(&p, &mut sm, &mut d);
        let names = if p.targets.is_empty() { ts.names() } else { p.targets.clone() };
        targets = names.into_iter().map(Some).collect();
    }
    let levels: Vec<Option<OptLevel>> = if o.all_levels && !output {
        vec![Some(OptLevel::O0), Some(OptLevel::O1), Some(OptLevel::O2), Some(OptLevel::Os)]
    } else {
        vec![o.opt]
    };
    let mut all_ok = true;
    let mut jsons = Vec::new();
    for t in &targets {
        for l in &levels {
            let gen_code = output || o.all_levels;
            let r = compile(o, t.clone(), *l, output, gen_code);
            if !r.ok {
                all_ok = false;
            }
            jsons.push(r.json);
        }
    }
    if o.json {
        println!("[{}]", jsons.join(","));
    }
    if all_ok {
        0
    } else {
        1
    }
}

fn compile(o: &Opts, target_name: Option<String>, level: Option<OptLevel>, write: bool, gen_code: bool) -> Result1 {
    let mut sm = SourceMap::new();
    let mut diags = Diags::new();
    let ppath = project_path(o);
    let proj: Project = match load_project(&ppath, &mut sm, &mut diags) {
        Some(p) => p,
        None => return fail(o, &diags, &sm, "?"),
    };
    for (k, v) in &proj.lints {
        if let Some(l) = lint_level_from(v) {
            diags.levels.insert(k.clone(), l);
        }
    }
    for (k, l) in &o.lints {
        diags.levels.insert(k.clone(), *l);
    }
    diags.deny_warnings = o.deny_warnings;
    let ts = load_targets(&proj, &mut sm, &mut diags);
    let tname = match target_name.clone().or_else(|| proj.default.clone()) {
        Some(t) => t,
        None => {
            let names = ts.names();
            if names.len() == 1 {
                names[0].clone()
            } else {
                diags.err("E1108", "ターゲットが決まりません（-t で指定するか、プロジェクトファイルに default を書いてください）", crate::diag::Span::none());
                return fail(o, &diags, &sm, "?");
            }
        }
    };
    let mut target = match ts.resolve(&tname, &mut diags) {
        Some(t) => t,
        None => return fail(o, &diags, &sm, &tname),
    };
    for (i, m) in &o.binds {
        target.binds.insert(i.clone(), m.clone());
    }
    if diags.has_errors() {
        return fail(o, &diags, &sm, &tname);
    }
    let mut build: BTreeMap<String, CfgVal> = BTreeMap::new();
    for (k, v) in &o.defines {
        let val = match v {
            None => CfgVal::Bool(true),
            Some(s) => {
                let t = s.replace('_', "");
                if let Ok(n) = t.parse::<i128>() {
                    CfgVal::Int(n)
                } else if let Some(h) = t.strip_prefix('$') {
                    CfgVal::Int(i128::from_str_radix(h, 16).unwrap_or(0))
                } else if s == "true" || s == "false" {
                    CfgVal::Bool(s == "true")
                } else {
                    CfgVal::Str(s.clone())
                }
            }
        };
        build.insert(k.clone(), val);
    }
    let lvl = level.or(o.opt).or_else(|| OptLevel::from_str(&proj.opt)).unwrap_or(OptLevel::O2);
    let title = format!("{}（{}）", tname, lvl.name());
    let step_limit = o.step_limit.unwrap_or(proj.step_limit);
    let std_dir = find_std_dir();
    if o.verbose && !o.json {
        eprintln!("ax65：プロジェクト {}、ターゲット {}、{}、標準ライブラリ {}", proj.name, tname, lvl.name(), std_dir.display());
    }
    let known = known_interfaces(&proj, &std_dir, &target);
    let loaded = match load_program(&proj, &target, &build, &known, &std_dir, &mut sm, &mut diags) {
        Some(l) => l,
        None => return fail(o, &diags, &sm, &title),
    };
    if diags.has_errors() {
        return fail(o, &diags, &sm, &title);
    }
    let nmos = target.is_nmos();
    let mut ctx = build_ctx(loaded, target.clone(), build, lvl, step_limit, known, sm, diags);
    ctx.check_program();
    if ctx.diags.has_errors() {
        return fail(o, &ctx.diags, &ctx.sm, &title);
    }
    if !gen_code {
        print_diags(o, &ctx.diags, &ctx.sm);
        if !o.json {
            eprintln!("ax65：{} の検査を終えました（警告 {} 件）", title, ctx.diags.warning_count());
        }
        return Result1 { ok: true, json: format!("{{\"target\":\"{}\",\"ok\":true,\"diagnostics\":{}}}", json_escape(&title), ctx.diags.to_json(&ctx.sm)) };
    }
    let mut g = generate(ctx, GenOpts { nmos });
    if g.ctx.diags.has_errors() {
        return fail(o, &g.ctx.diags, &g.ctx.sm, &title);
    }
    let linked = match link(&mut g) {
        Some(l) => l,
        None => return fail(o, &g.ctx.diags, &g.ctx.sm, &title),
    };
    if g.ctx.diags.has_errors() {
        return fail(o, &g.ctx.diags, &g.ctx.sm, &title);
    }
    let mut written: Vec<String> = Vec::new();
    if write {
        let dir = match &o.out_dir {
            Some(d) => PathBuf::from(d),
            None => proj.dir.join(&proj.out_dir).join(&tname),
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("エラー：出力先 {} を作れません：{}", dir.display(), e);
            return Result1 { ok: false, json: "{}".to_string() };
        }
        let mut formats = match &o.emit {
            Some(v) => v.clone(),
            None => target.output.clone().unwrap_or_else(|| vec!["bin".to_string(), "sym".to_string(), "lst".to_string()]),
        };
        if !o.no_report && !formats.iter().any(|f| f == "report") {
            formats.push("report".to_string());
        }
        let base = if proj.name.is_empty() { "out".to_string() } else { proj.name.clone() };
        let out = &linked.out;
        for f in &formats {
            let res = match f.as_str() {
                "bin" => write_file(&dir, &format!("{}.bin", base), &bin_bytes(out)),
                "woz" => write_file(&dir, &format!("{}.woz", base), woz_text(out, o.woz_line, o.woz_run).as_bytes()),
                "hex" => write_file(&dir, &format!("{}.hex", base), intel_hex(out).as_bytes()),
                "rom" => {
                    let roms = target.region("rom");
                    match roms.first() {
                        Some(r) => write_file(&dir, &format!("{}.rom", base), &rom_image(out, r.lo, r.hi)),
                        None => Err("rom 形式にはターゲット定義の region rom が必要です".to_string()),
                    }
                }
                "ptp" => write_file(&dir, &format!("{}.ptp", base), ptp_text(out).as_bytes()),
                "sym" => write_file(&dir, &format!("{}.sym", base), sym_text(out).as_bytes()),
                "lst" => write_file(&dir, &format!("{}.lst", base), lst_text(out, o.lst_plain, &title).as_bytes()),
                "report" => {
                    if o.no_report {
                        continue;
                    }
                    let rep = crate::report::build_report(&g, &linked, &title);
                    write_file(&dir, &format!("{}.report", base), rep.as_bytes())
                }
                _ => continue,
            };
            match res {
                Ok(p) => written.push(p),
                Err(e) => {
                    eprintln!("エラー：{}", e);
                    return Result1 { ok: false, json: "{}".to_string() };
                }
            }
        }
    }
    print_diags(o, &g.ctx.diags, &g.ctx.sm);
    let out = &linked.out;
    let code = out.code_end.saturating_sub(out.code_start);
    let data = out.data_end.saturating_sub(out.code_end);
    let vars = out.bss_end.saturating_sub(out.bss_start);
    if !o.json {
        eprintln!(
            "ax65：{} をコンパイルしました：コード {} バイト、定数 {} バイト、変数 {} バイト、スタック {} バイト（警告 {} 件）",
            title,
            code,
            data,
            vars,
            linked.stack.main + linked.stack.irq + linked.stack.nmi,
            g.ctx.diags.warning_count()
        );
        for w in &written {
            eprintln!("  → {}", w);
        }
    }
    let files: Vec<String> = written.iter().map(|w| format!("\"{}\"", json_escape(w))).collect();
    Result1 {
        ok: true,
        json: format!(
            "{{\"target\":\"{}\",\"ok\":true,\"code_bytes\":{},\"const_bytes\":{},\"var_bytes\":{},\"outputs\":[{}],\"diagnostics\":{}}}",
            json_escape(&title),
            code,
            data,
            vars,
            files.join(","),
            g.ctx.diags.to_json(&g.ctx.sm)
        ),
    }
}

fn cmd_explain(o: &Opts) -> i32 {
    let code = match o.positional.first() {
        Some(c) => c.clone(),
        None => {
            eprintln!("エラー：ax65 explain <番号> と書いてください（例：ax65 explain E0412）");
            return 2;
        }
    };
    let c2 = code.to_uppercase();
    let key = if c2.starts_with('E') { c2 } else { code.clone() };
    match crate::explain::explain(&key) {
        Some(t) => {
            print!("{}", t);
            0
        }
        None => {
            eprintln!("エラー：{} の説明はありません", code);
            1
        }
    }
}

fn cmd_targets(o: &Opts) -> i32 {
    let mut sm = SourceMap::new();
    let mut d = Diags::new();
    let p = match load_project(&project_path(o), &mut sm, &mut d) {
        Some(p) => p,
        None => {
            print_diags(o, &d, &sm);
            return 1;
        }
    };
    let ts = load_targets(&p, &mut sm, &mut d);
    for name in ts.names() {
        let raw = ts.raw.get(&name).cloned();
        let parent = raw.as_ref().and_then(|t| t.parent.clone());
        match ts.resolve(&name, &mut d) {
            Some(t) => {
                let ext = match parent {
                    Some(pp) => format!(" extends {}", pp),
                    None => String::new(),
                };
                let def = if p.default.as_deref() == Some(name.as_str()) { "（既定）" } else { "" };
                println!("{}{}{}", name, ext, def);
                println!("  cpu = {}、clock = {} Hz、stack = {}、run = {}", t.cpu.clone().unwrap_or_default(), t.clock.unwrap_or(0), t.stack.unwrap_or(0), t.run.clone().unwrap_or_default());
                if let Some(l) = t.load {
                    println!("  load = ${:04X}", l);
                }
                println!("  exit = {}、irq = {}、nmi = {}", exit_text(&t.exit), vec_text(&t.irq), vec_text(&t.nmi));
                let res: Vec<String> = t.reserved().iter().map(|r| if r.lo == r.hi { format!("${:04X}", r.lo) } else { format!("${:04X}..=${:04X}", r.lo, r.hi) }).collect();
                if !res.is_empty() {
                    println!("  reserve {}（ベクタの番地を含む）", res.join(", "));
                }
                for (k, v) in &t.regions {
                    let rs: Vec<String> = v.iter().map(|r| format!("${:04X}..=${:04X}", r.lo, r.hi)).collect();
                    println!("  region {} = {}", k, rs.join(", "));
                }
                if t.binds.is_empty() {
                    println!("  bind：なし");
                }
                for (k, v) in &t.binds {
                    println!("  bind {} = {}", k, v);
                }
                for (k, (ty, v)) in &t.consts {
                    println!("  const {}: {} = {:?}", k, ty, v);
                }
                for k in t.encodings.keys() {
                    println!("  encoding {}", k);
                }
            }
            None => {}
        }
    }
    print_diags(o, &d, &sm);
    if d.has_errors() {
        1
    } else {
        0
    }
}

fn exit_text(e: &Option<crate::project::Exit>) -> String {
    use crate::project::Exit;
    match e {
        Some(Exit::Rts) => "rts".to_string(),
        Some(Exit::Monitor(a)) => format!("monitor(${:04X})", a),
        Some(Exit::Halt) => "halt".to_string(),
        Some(Exit::Never) => "never".to_string(),
        None => "?".to_string(),
    }
}

fn vec_text(v: &Option<crate::project::VecMode>) -> String {
    use crate::project::VecMode;
    match v {
        Some(VecMode::Hw) => "hw".to_string(),
        Some(VecMode::Indirect(a)) => format!("indirect(${:04X})", a),
        Some(VecMode::Jump(a)) => format!("jump(${:04X})", a),
        Some(VecMode::None) | None => "none".to_string(),
    }
}

fn cmd_new(o: &Opts) -> i32 {
    let name = match o.positional.first() {
        Some(n) => n.clone(),
        None => {
            eprintln!("エラー：ax65 new <名前> と書いてください");
            return 2;
        }
    };
    let root = PathBuf::from(&name);
    if root.exists() {
        eprintln!("エラー：{} はすでにあります", name);
        return 1;
    }
    let files: Vec<(PathBuf, String)> = vec![
        (
            root.join("ax65.proj"),
            format!(
                "// ax65.proj（17.2節）\nproject {} {{\n    main       = \"src/{}.ax65\";\n    paths      = [\".\"];\n    target_dir = \"targets\";\n    targets    = [v2];\n    default    = v2;\n    opt        = O2;\n    out_dir    = \"build\";\n}}\n",
                name, name
            ),
        ),
        (
            root.join("targets").join("v2.target"),
            r#"// targets/v2.target（16章）— 6502-CPLD ボード＋HEXPAD（モニタ HEXMON）
target v2 {
    cpu   = Cpu::W65C02S;
    clock = 1_842_500;
    stack = 128;
    run   = ram;
    load  = $0200;                      // HEXMON で 0200 を表示して JMP キーで実行する
    exit  = rts;                        // rts でモニタへ戻る
    irq   = none;                       // 割り込みは使えない
    nmi   = none;
    region zp    = $20..=$FF;           // $00〜$1F はモニタが使う
    region stack = $0100..=$01FF;       // モニタと共用
    region ram   = $0200..=$7EFF;
    region io    = [$8000..=$800F,      // GPIO（6522 互換）
                    $8400..=$8403,      // UART（6551 互換）
                    $8800..=$880F];     // HEXPAD（表示とキー）
    reserve $8004..=$8007;              // T1（モニタの時間待ちが使う）
    reserve $800B;                      // ACR（同じ）
    output = [woz, bin, sym, lst];      // woz：HEXMON に打ち込むダンプ
    bind console = hw::acia_console;
    const LED_DIGITS: u8 = 6;
}
"#
                .to_string(),
        ),
        (
            root.join("src").join(format!("{}.ax65", name)),
            "// main を含むファイル\nuse console::putc;\n\nconst HELLO: str = p\"HELLO, AX65\";\n\nfn main() {\n    for c in HELLO { putc(c); }\n    putc('\\r');\n}\n".to_string(),
        ),
        (
            root.join("hw").join("acia_console.ax65"),
            "// console インターフェースの UART（6551 互換、$8400）による実装\nflags AciaStatus { Pe, Fe, Ovr, Rdrf, Tdre, Dcd, Dsr, Irq }\n\nimpl console {\n    io      DATA:   u8         @ $8400;\n    io(in)  STATUS: AciaStatus @ $8401;\n\n    fn putc(c: char) {\n        while !STATUS.contains(AciaStatus::Tdre) { }\n        DATA = c as u8;\n    }\n\n    fn poll() -> Option<char> {\n        if STATUS.contains(AciaStatus::Rdrf) { Some(DATA as char) } else { None }\n    }\n\n    fn getc() -> char {\n        loop {\n            if let Some(c) = poll() { return c; }\n        }\n    }\n}\n".to_string(),
        ),
    ];
    for (p, text) in files {
        if let Some(d) = p.parent() {
            if let Err(e) = std::fs::create_dir_all(d) {
                eprintln!("エラー：{} を作れません：{}", d.display(), e);
                return 1;
            }
        }
        if let Err(e) = std::fs::write(&p, text) {
            eprintln!("エラー：{} を書けません：{}", p.display(), e);
            return 1;
        }
        println!("作りました：{}", p.display());
    }
    println!("cd {} && ax65 build で、コンパイルできます", name);
    0
}

fn collect_sources(dir: &Path, out_dir: &Path, v: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for ent in rd.flatten() {
            let p = ent.path();
            if p.is_dir() {
                if p == out_dir {
                    continue;
                }
                collect_sources(&p, out_dir, v);
            } else if p.extension().and_then(|x| x.to_str()) == Some("ax65") {
                v.push(p);
            }
        }
    }
}

/// ax65 fmt：16進の書き方をそろえる（17.2節の fmt { hex = … }）
fn cmd_fmt(o: &Opts) -> i32 {
    let mut sm = SourceMap::new();
    let mut d = Diags::new();
    let p = match load_project(&project_path(o), &mut sm, &mut d) {
        Some(p) => p,
        None => {
            print_diags(o, &d, &sm);
            return 1;
        }
    };
    let want_dollar = p.fmt_hex != "c";
    let mut files = Vec::new();
    collect_sources(&p.dir, &p.dir.join(&p.out_dir), &mut files);
    files.sort();
    let mut bad = 0usize;
    for f in files {
        let text = match std::fs::read_to_string(&f) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let (toks, _) = tokenize_all(&text, 0);
        let chars: Vec<char> = text.chars().collect();
        let mut edits: Vec<(usize, usize, String)> = Vec::new();
        for t in &toks {
            if let Tok::Int(_, st) = &t.tok {
                let orig: String = chars[t.start..t.end].iter().collect();
                let new = match (st, want_dollar) {
                    (IntStyle::HexC, true) => Some(format!("${}", &orig[2..])),
                    (IntStyle::HexDollar, false) => Some(format!("0x{}", &orig[1..])),
                    _ => None,
                };
                if let Some(n) = new {
                    if o.fmt_check {
                        println!("{}:{}:{}：16進の書き方が fmt の設定（hex = {}）と違います（{}）", f.display(), t.span.line, t.span.col, p.fmt_hex, orig);
                    }
                    edits.push((t.start, t.end, n));
                }
            }
        }
        if edits.is_empty() {
            continue;
        }
        bad += edits.len();
        if !o.fmt_check {
            let mut out = String::new();
            let mut pos = 0usize;
            for (s, e, n) in &edits {
                let before: String = chars[pos..*s].iter().collect();
                out.push_str(&before);
                out.push_str(n);
                pos = *e;
            }
            let rest: String = chars[pos..].iter().collect();
            out.push_str(&rest);
            if let Err(e) = std::fs::write(&f, out) {
                eprintln!("エラー：{} を書けません：{}", f.display(), e);
                return 1;
            }
            println!("そろえました：{}（{} か所）", f.display(), edits.len());
        }
    }
    println!("補足：配列の型の書き方（fmt {{ array = … }}）の整形は未実装です");
    if o.fmt_check && bad > 0 {
        1
    } else {
        0
    }
}
