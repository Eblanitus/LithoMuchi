//! xtask: документация параметров (T12/T04) и генерация шаблонов Э2
//! (T72–T75). Всё на std, без внешних крейтов: бандл уходит в workspace
//! Horizon, где лишние зависимости запрещены (план 5.4).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use litho_core::{GenVersion, Mode, Params};

/// Версия генератора, для которой сейчас собираются паки (план 6.8).
const GEN_VERSION: GenVersion = GenVersion::V0;
/// Версия самого мода в `[mod] version` манифеста.
const MOD_VERSION: &str = "0.2.0";
/// Ревизия Horizon из раздела 5 плана; сверить на Windows (T70).
const HORIZON_REV: &str = "66f3981f522b24c250b21523a70080965b940424";
/// Пять полей, которые подменяем в MVP (точка №2 плана 10.1).
const FIELDS: [&str; 5] = [
    "continents",
    "erosion",
    "ridges",
    "temperature",
    "vegetation",
];

fn repo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn mod_dir() -> PathBuf {
    repo_dir().join("mod")
}

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

// ---------------------------------------------------------------- Э2, T72–T75

fn read_template(rel: &str) -> Result<String, String> {
    let path = mod_dir().join(rel);
    fs::read_to_string(&path).map_err(|e| format!("нет шаблона {}: {e}", path.display()))
}

fn unresolved(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'@' {
            i += 1;
            continue;
        }
        let Some(j) = text[i + 1..].find('@') else {
            i += 1;
            continue;
        };
        let token = &text[i + 1..i + 1 + j];
        if !token.is_empty()
            && token
                .chars()
                .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
        {
            return Some(format!("@{token}@"));
        }
        i += 1;
    }
    None
}

fn render(src: &str, vars: &[(&str, &str)]) -> Result<String, String> {
    let mut out = src.to_string();
    for &(key, value) in vars {
        out = out.replace(&format!("@{key}@"), value);
    }
    if let Some(token) = unresolved(&out) {
        return Err(format!("неподставленный плейсхолдер {token}"));
    }
    Ok(out)
}

fn write_file(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("не могу создать {}: {e}", parent.display()))?;
    }
    fs::write(path, text).map_err(|e| format!("не могу записать {}: {e}", path.display()))
}

/// Один датапак: 1 метаданные + 5 переопределений + 5 фиктивных шумов.
fn make_pack(dir: &Path, label: &str, noise_prefix: &str, noise_dir: &str) -> Result<(), String> {
    let mcmeta = read_template("packs/pack.mcmeta.in")?;
    let density = read_template("packs/density_function.json.in")?;
    let noise = read_template("packs/noise.json.in")?;

    write_file(
        &dir.join("pack.mcmeta"),
        &render(
            &mcmeta,
            &[
                ("VERSION", GEN_VERSION.as_str()),
                ("MODE", noise_dir),
                ("LABEL", label),
            ],
        )?,
    )?;
    for field in FIELDS {
        let noise_id = format!("{noise_prefix}/{field}");
        let df = render(&density, &[("NOISE", noise_id.as_str())])?;
        write_file(
            &dir.join(format!(
                "data/minecraft/worldgen/density_function/overworld/{field}.json"
            )),
            &df,
        )?;
        write_file(
            &dir.join(format!(
                "data/litospherity/worldgen/noise/{noise_dir}/{field}.json"
            )),
            &noise,
        )?;
    }
    if noise_dir != "smoke" {
        let mode = noise_dir.rsplit('/').next().ok_or("нет режима")?;
        fs::copy(
            mod_dir().join(format!("assets/{mode}.png")),
            dir.join("pack.png"),
        )
        .map_err(|e| e.to_string())?;
        write_file(
            &dir.join(format!(
                "data/litospherity/worldgen/density_function/{noise_dir}/active.json"
            )),
            "0.0\n",
        )?;
        let lava_id = format!("{noise_prefix}/lava");
        write_file(
            &dir.join(format!(
                "data/litospherity/worldgen/density_function/{noise_dir}/lava.json"
            )),
            &format!(
                "{{\"type\":\"minecraft:noise\",\"noise\":\"{lava_id}\",\"xz_scale\":64.0,\"y_scale\":40.0}}\n"
            ),
        )?;
        write_file(
            &dir.join(format!(
                "data/litospherity/worldgen/noise/{noise_dir}/lava.json"
            )),
            &noise,
        )?;
    }
    println!("пак: {}", dir.display());
    Ok(())
}

fn cmd_pack(out: &Path) -> Result<(), String> {
    fs::create_dir_all(out).map_err(|e| format!("не могу создать {}: {e}", out.display()))?;
    for mode in Mode::ALL {
        let name = format!("litospherity_{}_{}", GEN_VERSION.as_str(), mode.as_str());
        let noise_dir = format!("{}/{}", GEN_VERSION.as_str(), mode.as_str());
        make_pack(
            &out.join(&name),
            if mode == Mode::Gameplay {
                "Игровой мир — просторные материки и климат, удобный масштаб"
            } else {
                "Научный мир — крупные материки и протяжённые климатические пояса"
            },
            &format!("litospherity:{noise_dir}"),
            noise_dir.as_str(),
        )?;
    }
    // Smoke-пак DoD Э2: всюду суша с постоянной континентальностью.
    make_pack(
        &out.join("litospherity_smoke"),
        "smoke-тест точки №1 (всюду суша)",
        "litospherity:smoke",
        "smoke",
    )?;
    Ok(())
}

fn copy_dir(src: &Path, dst: &Path) -> Result<usize, String> {
    fs::create_dir_all(dst).map_err(|e| format!("не могу создать {}: {e}", dst.display()))?;
    let mut count = 0;
    let entries = fs::read_dir(src).map_err(|e| format!("не читаю {}: {e}", src.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "target" || name == ".git" || name.ends_with(".rs.bk") {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        if from.is_dir() {
            count += copy_dir(&from, &to)?;
        } else {
            fs::copy(&from, &to).map_err(|e| format!("копия {}: {e}", from.display()))?;
            count += 1;
        }
    }
    Ok(count)
}

fn cmd_bundle(out: &Path) -> Result<(), String> {
    fs::create_dir_all(out).map_err(|e| format!("не могу создать {}: {e}", out.display()))?;
    let manifest = read_template("horizon-mixin.toml.in")?;
    write_file(
        &out.join("horizon-mixin.toml"),
        &render(
            &manifest,
            &[("VERSION", MOD_VERSION), ("HORIZON_REV", HORIZON_REV)],
        )?,
    )?;
    for (src, dst) in [
        ("glue/noise_hook.rs.in", "glue/noise_hook.rs"),
        ("glue/packs_hook.rs.in", "glue/packs_hook.rs"),
        ("glue/packs_guard.rs.in", "glue/packs_guard.rs"),
        ("glue/lava_settings.rs.in", "glue/lava_settings.rs"),
        ("glue/litospherity_glue.rs.in", "glue/litospherity_glue.rs"),
    ] {
        let text = read_template(src)?;
        if let Some(token) = unresolved(&text) {
            return Err(format!("{src}: неподставленный плейсхолдер {token}"));
        }
        write_file(&out.join(dst), &text)?;
    }
    let n = copy_dir(
        &repo_dir().join("crates/litho-core"),
        &out.join("crates/litho-core"),
    )?;
    cmd_pack(&out.join("packs"))?;
    make_packs_crate(out)?;
    write_file(
        &out.join("README.txt"),
        "Бандл собран xtask, НЕ проверен лаунчером Horizon (T70/T72).\n\
         Команда на Windows: horizon-launcher native-build <эта папка> --instance <имя> --trust-native-code\n\
         Сначала включите пак litospherity_smoke: мир должен быть всюду сушей.\n\
         Подробности: docs/HORIZON.md в репозитории.\n",
    )?;
    println!("бандл: {} (исходников ядра: {n})", out.display());
    Ok(())
}

/// Embed only the two player modes in a dependency-free crate. Includes are
/// legal inside bundle crates (support modules cannot use include_bytes!).
fn make_packs_crate(out: &Path) -> Result<(), String> {
    let dst = out.join("crates/litho-packs");
    write_file(
        &dst.join("Cargo.toml"),
        "[package]\nname = \"litho-packs\"\nversion = \"0.2.0\"\nedition = \"2024\"\nrust-version = \"1.85\"\nlicense = \"MIT OR Apache-2.0\"\n[dependencies]\n",
    )?;
    let mut source = String::from("const FILES: &[(&str, &str, &[u8])] = &[\n");
    for (mode, title) in [
        ("gameplay", "Litospherity - Игровой мир"),
        ("scientific", "Litospherity - Научный мир"),
    ] {
        let pack = out.join(format!("packs/litospherity_v0_{mode}"));
        let data = dst.join(format!("data/{mode}"));
        copy_dir(&pack, &data)?;
        let mut files = Vec::new();
        collect_files(&data, &data, &mut files)?;
        files.sort();
        for rel in files {
            source.push_str(&format!(
                "    ({title:?}, {rel:?}, include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/data/{mode}/{rel}\"))),\n"
            ));
        }
    }
    source.push_str("];\n");
    source.push_str(&read_template("packs_installer.rs")?);
    write_file(&dst.join("src/lib.rs"), &source)
}

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<String>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            collect_files(root, &path, files)?;
        } else {
            files.push(
                path.strip_prefix(root)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    Ok(())
}

fn check_no_placeholders(dir: &Path) -> Result<usize, String> {
    let mut count = 0;
    let entries = fs::read_dir(dir).map_err(|e| format!("не читаю {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            count += check_no_placeholders(&path)?;
            continue;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !(name.ends_with(".json")
            || name.ends_with(".toml")
            || name.ends_with(".rs")
            || name.ends_with(".mcmeta"))
        {
            continue;
        }
        let text =
            fs::read_to_string(&path).map_err(|e| format!("не читаю {}: {e}", path.display()))?;
        if let Some(token) = unresolved(&text) {
            return Err(format!("{}: остался {token}", path.display()));
        }
        if name.ends_with(".json") || name.ends_with(".mcmeta") {
            let trimmed = text.trim();
            if trimmed != "0.0" && (!trimmed.starts_with('{') || !trimmed.ends_with('}')) {
                return Err(format!("{}: не похоже на JSON", path.display()));
            }
        }
        count += 1;
    }
    Ok(count)
}

fn cmd_bundle_check(out: &Path) -> Result<(), String> {
    let manifest = out.join("horizon-mixin.toml");
    let text =
        fs::read_to_string(&manifest).map_err(|e| format!("нет {}: {e}", manifest.display()))?;
    if !text.contains("dependent = \"horizon-core\"") {
        return Err("манифест не подключает litho-core к horizon-core".into());
    }
    for rel in [
        "glue/noise_hook.rs",
        "glue/litospherity_glue.rs",
        "crates/litho-core/Cargo.toml",
        "crates/litho-core/src/lib.rs",
    ] {
        if !out.join(rel).is_file() {
            return Err(format!("в бандле нет {rel}"));
        }
    }
    for mode in Mode::ALL {
        let pack = out.join(format!(
            "packs/litospherity_{}_{}",
            GEN_VERSION.as_str(),
            mode.as_str()
        ));
        for field in FIELDS {
            let f = pack.join(format!(
                "data/minecraft/worldgen/density_function/overworld/{field}.json"
            ));
            if !f.is_file() {
                return Err(format!("в паке {} нет {field}", pack.display()));
            }
        }
    }
    let smoke = out.join("packs/litospherity_smoke/pack.mcmeta");
    if !smoke.is_file() {
        return Err("нет smoke-пака (DoD Э2)".into());
    }
    let n = check_no_placeholders(out)?;
    println!("Э2: бандл проверен, файлов {n}, плейсхолдеров нет");
    println!("Э2: сборка лаунчером на Windows НЕ подтверждена (T70/T72)");
    Ok(())
}

fn out_arg(args: &[String]) -> Result<PathBuf, String> {
    for pair in args.windows(2) {
        if pair[0] == "--out" {
            return Ok(PathBuf::from(pair[1].as_str()));
        }
    }
    if args.last().map(String::as_str) == Some("--out") {
        return Err("--out требует путь".into());
    }
    Ok(repo_dir().join("out/bundle"))
}

fn usage() -> &'static str {
    "xtask — задачи разработки\n\n\
     params-doc                    таблица параметров v0 в Markdown (в stdout)\n\
     params-hash                   хэши параметров для scientific и gameplay\n\
     pack  [--out ПАПКА]           три датапака 26.3 (2 режима + smoke)\n\
     bundle [--out ПАПКА]          бандл нативного мода из шаблонов mod/\n\
     bundle-check [--out ПАПКА]    жёсткая проверка бандла (используется в CI)\n\
     help                          эта справка\n"
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("params-doc") => cmd_params_doc(),
        Some("params-hash") => cmd_params_hash(),
        Some("pack") => cmd_pack(&out_arg(&args[1..])?)?,
        Some("bundle") => cmd_bundle(&out_arg(&args[1..])?)?,
        Some("bundle-check") => cmd_bundle_check(&out_arg(&args[1..])?)?,
        Some("help") | Some("--help") | Some("-h") | None => print!("{text}", text = usage()),
        Some(other) => return Err(format!("неизвестная команда '{other}'")),
    }
    Ok(())
}
