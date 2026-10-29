// 出力形式（17.6節）：bin woz hex rom ptp sym lst
use crate::asm6502::{format_list_line, AsmOutput};
use std::collections::BTreeMap;
use std::path::Path;

pub fn bin_bytes(out: &AsmOutput) -> Vec<u8> {
    let start = out.code_start;
    let end = out.data_end;
    let mut v = Vec::new();
    let mut a = start;
    while a < end {
        v.push(*out.image.get(&a).unwrap_or(&0));
        a += 1;
    }
    v
}

pub fn woz_text(out: &AsmOutput, per_line: usize, run: bool) -> String {
    let bytes = bin_bytes(out);
    let per = if per_line == 0 { 8 } else { per_line };
    let mut s = String::new();
    for (k, chunk) in bytes.chunks(per).enumerate() {
        let addr = out.code_start as usize + k * per;
        let b: Vec<String> = chunk.iter().map(|x| format!("{:02X}", x)).collect();
        s.push_str(&format!("{:04X}: {}\n", addr, b.join(" ")));
    }
    if run {
        s.push_str(&format!("{:X}R\n", out.code_start));
    }
    s
}

fn segments(image: &BTreeMap<u32, u8>) -> Vec<(u32, Vec<u8>)> {
    let mut segs: Vec<(u32, Vec<u8>)> = Vec::new();
    for (a, b) in image {
        let mut extend = false;
        if let Some((start, v)) = segs.last() {
            if *start + v.len() as u32 == *a {
                extend = true;
            }
        }
        if extend {
            if let Some(last) = segs.last_mut() {
                last.1.push(*b);
            }
        } else {
            segs.push((*a, vec![*b]));
        }
    }
    segs
}

pub fn intel_hex(out: &AsmOutput) -> String {
    let mut s = String::new();
    for (start, data) in segments(&out.image) {
        for (k, chunk) in data.chunks(16).enumerate() {
            let addr = start as usize + k * 16;
            let mut sum: u32 = chunk.len() as u32 + ((addr >> 8) & 0xFF) as u32 + (addr & 0xFF) as u32;
            let mut line = format!(":{:02X}{:04X}00", chunk.len(), addr & 0xFFFF);
            for b in chunk {
                line.push_str(&format!("{:02X}", b));
                sum += *b as u32;
            }
            let cs = ((0x100 - (sum & 0xFF)) & 0xFF) as u8;
            line.push_str(&format!("{:02X}\n", cs));
            s.push_str(&line);
        }
    }
    s.push_str(":00000001FF\n");
    s
}

pub fn rom_image(out: &AsmOutput, lo: u32, hi: u32) -> Vec<u8> {
    let mut v = vec![0xFFu8; (hi - lo + 1) as usize];
    for (a, b) in &out.image {
        if *a >= lo && *a <= hi {
            v[(*a - lo) as usize] = *b;
        }
    }
    v
}

/// KIM-1 の紙テープ形式
pub fn ptp_text(out: &AsmOutput) -> String {
    let mut s = String::new();
    let mut count = 0u32;
    for (start, data) in segments(&out.image) {
        for (k, chunk) in data.chunks(24).enumerate() {
            let addr = start as usize + k * 24;
            let mut sum: u32 = chunk.len() as u32 + ((addr >> 8) & 0xFF) as u32 + (addr & 0xFF) as u32;
            let mut line = format!(";{:02X}{:04X}", chunk.len(), addr & 0xFFFF);
            for b in chunk {
                line.push_str(&format!("{:02X}", b));
                sum += *b as u32;
            }
            line.push_str(&format!("{:04X}\n", sum & 0xFFFF));
            s.push_str(&line);
            count += 1;
        }
    }
    let sum = (count >> 8) + (count & 0xFF);
    s.push_str(&format!(";00{:04X}{:04X}\n", count, sum & 0xFFFF));
    s
}

pub fn sym_text(out: &AsmOutput) -> String {
    let mut v: Vec<(u32, String)> = out
        .symbols
        .iter()
        .filter(|(k, _)| !k.contains('~'))
        .map(|(k, a)| (*a, k.clone()))
        .collect();
    v.sort();
    let mut s = String::from("; AX65 記号表（名前 = アドレス）\n");
    for (a, k) in v {
        s.push_str(&format!("{} = ${:04X}\n", k, a));
    }
    s
}

pub fn lst_text(out: &AsmOutput, plain: bool, title: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("; {}\n", title));
    s.push_str("addr  bytes      cyc    命令             ; 説明\n");
    for l in &out.listing {
        s.push_str(&format_list_line(l, plain));
        s.push('\n');
    }
    s
}

pub fn write_file(dir: &Path, name: &str, data: &[u8]) -> Result<String, String> {
    let p = dir.join(name);
    std::fs::write(&p, data).map_err(|e| format!("{} に書けません：{}", p.display(), e))?;
    Ok(p.display().to_string())
}
