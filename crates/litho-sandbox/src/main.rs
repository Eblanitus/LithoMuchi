//! Песочница Litospherity: `render`, `probe`, `layers`, `help` (план 12.2).
//!
//! Быстрый старт:
//!   cargo run -p litho-sandbox --release -- render --seed 42 --mode gameplay \
//!       --layers plates,boundaries,jacobian --size 512 --out out/seed42
//!   cargo run -p litho-sandbox --release -- probe --seed 42 --at 1200,-3400

mod palette;
mod png;
mod render;

use std::path::Path;

use litho_core::{GenVersion, Mode, Params};
use render::{LAYERS, RenderOptions};

fn value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1).cloned())
}

fn parse_pair(text: &str) -> Result<(f64, f64), String> {
    let mut parts = text.split(',');
    let a = parts.next().unwrap_or("").trim();
    let b = parts
        .next()
        .ok_or("нужны две координаты через запятую")?
        .trim();
    let x = a
        .parse::<f64>()
        .map_err(|e| format!("координата '{a}': {e}"))?;
    let z = b
        .parse::<f64>()
        .map_err(|e| format!("координата '{b}': {e}"))?;
    Ok((x, z))
}

fn options_from(args: &[String]) -> Result<RenderOptions, String> {
    let seed = match value(args, "--seed") {
        Some(text) => text.parse::<i64>().map_err(|e| format("--seed: {e}"))?,
        None => 42,
    };
    let mode = match value(args, "--mode") {
        Some(text) => Mode::parse(&text).ok_or_else(|| format!("неизвестный режим '{text}'"))?,
        None => Mode::Gameplay,
    };
    let version = match value(args, "--version") {
        Some(text) => {
            GenVersion::parse(&text).ok_or_else(|| format!("неизвестная версия '{text}'"))?
        }
        None => GenVersion::V0,
    };
    let (center_x, center_z) = match value(args, "--center") {
        Some(text) => parse_pair(&text)?,
        None => (0.0, 0.0),
    };
    let size = match value(args, "--size") {
        Some(text) => text.parse::<usize>().map_err(|e| format!("--size: {e}"))?,
        None => 512,
    };
    let bpp = match value(args, "--bpp") {
        Some(text) => text.parse::<f64>().map_err(|e| format!("--bpp: {e}"))?,
        // 12.2: обзор планеты 2048², 64 блока на пиксель в научном режиме
        // и 16 в геймплейном — одна и та же область планеты.
        None => match mode {
            Mode::Scientific => 64.0,
            Mode::Gameplay => 16.0,
        },
    };
    if size == 0 || !(bpp > 0.0) {
        return Err("--size должен быть > 0, --bpp строго > 0".to_string());
    }
    Ok(RenderOptions {
        seed,
        mode,
        version,
        center_x,
        center_z,
        size,
        bpp,
    })
}

fn cmd_render(args: &[String]) -> Result<(), String> {
    let opts = options_from(args)?;
    if let Some(path) = value(args, "--path") {
        if path != "reference" && path != "fast" {
            return Err(format!(
                "--path: ожидалось reference|fast, получено '{path}'"
            ));
        }
        if path == "fast" {
            println!("внимание: решётка T13 ещё не реализована, используется эталонный путь");
        }
    }
    let layers = match value(args, "--layers") {
        Some(text) if text == "all" => LAYERS.to_vec(),
        Some(text) => text.split(',').map(|s| s.trim().to_string()).collect(),
        None => vec!["plates".to_string()],
    };
    let out = value(args, "--out").unwrap_or_else(|| "out".to_string());
    let dir = Path::new(&out);
    std::fs::create_dir_all(dir).map_err(|e| format!("не могу создать '{out}': {e}"))?;
    let params_hash = Params::defaults(opts.version, opts.mode).params_hash();

    for layer in layers {
        let image = render::render_layer(&opts, &layer)?;
        let png_path = dir.join(format!("{layer}.png"));
        png::write_png(&png_path, &image).map_err(|e| {
            let path = png_path.display();
            format!("запись '{path}': {e}")
        })?;
        let json_path = dir.join(format!("{layer}.json"));
        std::fs::write(&json_path, render::metadata(&opts, &layer, params_hash)).map_err(|e| {
            let path = json_path.display();
            format!("запись '{path}': {e}")
        })?;
        println!("{layer}: {}", png_path.display());
    }
    Ok(())
}

fn cmd_probe(args: &[String]) -> Result<(), String> {
    let opts = options_from(args)?;
    let at = value(args, "--at").ok_or("нужно --at x,z")?;
    let (x, z) = parse_pair(&at)?;
    let text = render::probe_text(&opts, x, z)?;
    println!("{text}");
    Ok(())
}

fn usage() -> &'static str {
    "Litospherity sandbox (этап Э1a/Э1b)\n\n\
     Команды:\n\
       render   карты PNG в папку --out\n\
       probe    все доступные поля в точке --at x,z\n\
       layers   список слоёв\n\
       help     эта справка\n\n\
     Общие флаги: --seed N (42), --mode scientific|gameplay (gameplay),\n\
       --version v0|v1 (v0), --center x,z (0,0), --size N (512),\n\
       --bpp N (64 научный / 16 геймплейный), --path reference|fast,\n\
       --layers plates|all|список через запятую, --out ПАПКА\n\n\
     Пример:\n\
       cargo run -p litho-sandbox --release -- render --seed 42 --mode gameplay \\\n\
         --layers plates,boundaries,jacobian --size 512 --out out/seed42\n"
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first() else {
        print!("{text}", text = usage());
        return Ok(());
    };
    match command.as_str() {
        "render" => cmd_render(&args[1..]),
        "probe" => cmd_probe(&args[1..]),
        "layers" => {
            for layer in LAYERS {
                println!("{layer}");
            }
            Ok(())
        }
        "help" | "--help" | "-h" => {
            print!("{text}", text = usage());
            Ok(())
        }
        other => Err(format!("неизвестная команда '{other}'; попробуйте help")),
    }
}
