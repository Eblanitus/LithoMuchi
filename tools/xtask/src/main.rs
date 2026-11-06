//! xtask: пока только документация параметров (T12/T04).
//! Позже здесь появятся `bundle`, `golden`, генерация датапаков (T73–T75).

use std::collections::BTreeMap;

use litho_core::{GenVersion, Mode, Params};

fn canonical_map(params: &Params) -> BTreeMap<String, String> {
    params
        .canonical_text()
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn cmd_params_doc() {
    let scientific = Params::defaults(GenVersion::V0, Mode::Scientific);
    let gameplay = Params::defaults(GenVersion::V0, Mode::Gameplay);
    let a = canonical_map(&scientific);
    let b = canonical_map(&gameplay);
    println!("# Параметры генератора v0");
    println!();
    println!("Сгенерировано `cargo run -p xtask -- params-doc > docs/PARAMS.md`.");
    println!("Источник правды — `Params::defaults` в `crates/litho-core/src/params.rs`.");
    println!();
    println!("| Параметр | scientific | gameplay |");
    println!("|---|---|---|");
    for (key, va) in &a {
        let vb = b.get(key).map(String::as_str).unwrap_or("—");
        println!("| `{key}` | `{va}` | `{vb}` |");
    }
    println!();
    println!(
        "params_hash: scientific = {:016x}, gameplay = {:016x}",
        scientific.params_hash(),
        gameplay.params_hash()
    );
}

fn cmd_params_hash() {
    for mode in Mode::ALL {
        let params = Params::defaults(GenVersion::V0, mode);
        println!(
            "{name} {hash:016x}",
            name = mode.as_str(),
            hash = params.params_hash()
        );
    }
}

fn usage() -> &'static str {
    "xtask — задачи разработки\n\n\
     params-doc    таблица параметров v0 в Markdown (в stdout)\n\
     params-hash   хэши параметров для scientific и gameplay\n\
     help          эта справка\n"
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("params-doc") => cmd_params_doc(),
        Some("params-hash") => cmd_params_hash(),
        Some("help") | Some("--help") | Some("-h") | None => print!("{text}", text = usage()),
        Some(other) => return Err(format!("неизвестная команда '{other}'")),
    }
    Ok(())
}
