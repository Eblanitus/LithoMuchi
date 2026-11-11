//! xtask: документация параметров (T12/T04) и генерация шаблонов Э2
//! (T72–T75). Всё на std, без внешних крейтов: бандл уходит в workspace
//! Horizon, где лишние зависимости запрещены (план 5.4).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use litho_core::{GenVersion, Mode, Params};

/// Версия генератора, для которой сейчас собираются паки (план 6.8).
const GEN_VERSION: GenVersion = GenVersion::V0;
/// v0 остаётся в бандле неизменным; рядом собираются рабочие паки Э3.5.
const PACK_VERSIONS: [GenVersion; 2] = [GenVersion::V0, GenVersion::V1Dev];
/// Версия самого мода в `[mod] version` манифеста.
const MOD_VERSION: &str = "0.1.0";
/// Ревизия Horizon: единственное закрепление (К1, шаг 1). Подтверждена
/// автором и проверена на настоящих исходниках в CI (horizon-core-check).
const HORIZON_REV: &str = "66f3981f522b24c250b21523a70080965b940424";
/// Пять полей, которые подменяем в MVP (точка №2 плана 10.1).
/// v1-dev дополнительно переопределяет `overworld/ridges_folded` полем `pv`
/// (T141/I2-lite) и `offset` своей высотой (T144/полная I2). `factor` и
/// `jaggedness` на первый тест НЕ переопределяются (шаг 2): ваниль посчитает
/// их сама из наших C/E/PV. Свою таблицу в шкале ванили вернуть позже.
/// v0 остаётся ровно с пятью полями.
const FIELDS: [&str; 5] = [
    "continents",
    "erosion",
    "ridges",
    "temperature",
    "vegetation",
];
/// v1-dev заменяет PV (`ridges_folded`) и offset; factor/jaggedness считает
/// сама ваниль на первом тесте в игре (К2, шаг 2 плана готовности).
const I2_FIELDS: [&str; 2] = ["offset", "pv"];
const VANILLA_COMPUTED_FIELDS: [&str; 2] = ["factor", "jaggedness"];

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

fn cmd_params_doc(version: GenVersion) {
    let scientific = Params::defaults(version, Mode::Scientific);
    let gameplay = Params::defaults(version, Mode::Gameplay);
    let a = canonical_map(&scientific);
    let b = canonical_map(&gameplay);
    println!("# Параметры генератора {}", version.as_str());
    println!();
    let out_file = if version.as_str() == "v0" {
        "docs/PARAMS.md"
    } else {
        "docs/PARAMS-V1-DEV.md"
    };
    let version_arg = if version.as_str() == "v0" {
        String::new()
    } else {
        format!(" --version {}", version.as_str())
    };
    println!("Сгенерировано `cargo run -p xtask -- params-doc{version_arg} > {out_file}`.");
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

fn cmd_params_hash(version: GenVersion) {
    for mode in Mode::ALL {
        let params = Params::defaults(version, mode);
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

/// Один датапак: метаданные, переопределения density_function и фиктивные
/// шумы. v0/smoke — пять полей; v1-dev добавляет pv/offset. factor и
/// jaggedness на тест №2 не трогаем (К2/шаг 2), их считает ваниль.
fn make_pack(
    dir: &Path,
    version: GenVersion,
    label: &str,
    noise_prefix: &str,
    noise_dir: &str,
) -> Result<(), String> {
    let mcmeta = read_template("packs/pack.mcmeta.in")?;
    let density = read_template("packs/density_function.json.in")?;
    let noise = read_template("packs/noise.json.in")?;

    write_file(
        &dir.join("pack.mcmeta"),
        &render(
            &mcmeta,
            &[
                ("VERSION", version.as_str()),
                ("MODE", noise_dir),
                ("LABEL", label),
            ],
        )?,
    )?;
    let mut fields: Vec<&str> = FIELDS.to_vec();
    if version.is_water_relief_dev() {
        // Пак заменяет `overworld/offset` ЦЕЛИКОМ: в ванильном графе внутри
        // уже сидит −0,50375, поэтому наш шум должен быть y/128 − 1.
        fields.push("pv");
        fields.push("offset");
        // factor/jaggedness не переопределяем: ваниль берёт их из сплайнов
        // по нашим C/E/PV. Таблица в шкале ванили остаётся в ядре до шага 5.
    }
    for field in fields {
        // В ванили PV — именованная функция `ridges_folded`, на неё по имени
        // ссылаются offset/factor/jaggedness. Сам шум называется `pv`.
        // Пак заменяет ridges_folded и offset; factor/jaggedness остаются
        // ванильными и берут уже нашу PV через общий граф.
        let df_name = if field == "pv" {
            "ridges_folded"
        } else {
            field
        };
        let noise_id = format!("{noise_prefix}/{field}");
        let df = render(&density, &[("NOISE", noise_id.as_str())])?;
        write_file(
            &dir.join(format!(
                "data/minecraft/worldgen/density_function/overworld/{df_name}.json"
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
    println!("пак: {}", dir.display());
    Ok(())
}

/// Удаляет ТОЛЬКО паки, которыми владеет xtask: после смены состава полей
/// старые `factor.json`/`jaggedness.json` не должны попасть в `.hnmod`.
fn reset_dir(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_dir_all(path).map_err(|e| format!("не могу очистить {}: {e}", path.display()))?;
    }
    fs::create_dir_all(path).map_err(|e| format!("не могу создать {}: {e}", path.display()))
}

fn owned_packs(out: &Path) -> Vec<PathBuf> {
    let mut names = Vec::new();
    for version in PACK_VERSIONS {
        for mode in Mode::ALL {
            names.push(out.join(format!("litospherity_{}_{}", version.as_str(), mode.as_str())));
        }
    }
    names.push(out.join("litospherity_smoke"));
    names
}

fn cmd_pack(out: &Path) -> Result<(), String> {
    fs::create_dir_all(out).map_err(|e| format!("не могу создать {}: {e}", out.display()))?;
    for pack in owned_packs(out) {
        reset_dir(&pack)?;
    }
    for version in PACK_VERSIONS {
        for mode in Mode::ALL {
            let name = format!("litospherity_{}_{}", version.as_str(), mode.as_str());
            let noise_dir = format!("{}/{}", version.as_str(), mode.as_str());
            make_pack(
                &out.join(&name),
                version,
                if version == GenVersion::V1Dev {
                    "Э3.5: рабочая версия, не релиз"
                } else {
                    "поля Litospherity"
                },
                &format!("litospherity:{noise_dir}"),
                noise_dir.as_str(),
            )?;
        }
    }
    // Smoke-пак DoD Э2: всюду суша с постоянной континентальностью.
    make_pack(
        &out.join("litospherity_smoke"),
        GEN_VERSION,
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
    // Чистим только свои подпапки: в бандл не должны попасть удалённые из
    // ядра файлы или паки прошлого состава полей.
    reset_dir(&out.join("glue"))?;
    reset_dir(&out.join("crates/litho-core"))?;
    reset_dir(&out.join("packs"))?;
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
    write_file(
        &out.join("README.txt"),
        "Бандл собран xtask; план проверен horizon-mixins в CI, но НЕ собран лаунчером.\n\
         Команда на Windows: horizon-launcher native-build <эта папка> --instance <имя> --trust-native-code\n\
         Smoke: включите только litospherity_smoke, мир должен быть всюду сушей.\n\
         Тест №2: включите только litospherity_v1-dev_gameplay, запишите сид и номер сборки.\n\
         Актуальное состояние: docs/PLAN-STATUS-2026-11-11.md в репозитории.\n",
    )?;
    println!("бандл: {} (исходников ядра: {n})", out.display());
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
            if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
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
    for version in PACK_VERSIONS {
        for mode in Mode::ALL {
            let pack = out.join(format!(
                "packs/litospherity_{}_{}",
                version.as_str(),
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
            let df_path = |df_name: &str| {
                pack.join(format!(
                    "data/minecraft/worldgen/density_function/overworld/{df_name}.json"
                ))
            };
            let noise_path = |field: &str| {
                pack.join(format!(
                    "data/litospherity/worldgen/noise/{}/{}/{field}.json",
                    version.as_str(),
                    mode.as_str()
                ))
            };
            if version.is_water_relief_dev() {
                for field in I2_FIELDS {
                    let df_name = if field == "pv" { "ridges_folded" } else { field };
                    if !df_path(df_name).is_file() || !noise_path(field).is_file() {
                        return Err(format!("в паке {} нет I2-поля {field}", pack.display()));
                    }
                }
                for field in VANILLA_COMPUTED_FIELDS {
                    if df_path(field).exists() || noise_path(field).exists() {
                        return Err(format!(
                            "пак {} не должен переопределять {field} до теста №2",
                            pack.display()
                        ));
                    }
                }
            } else {
                for field in I2_FIELDS {
                    let df_name = if field == "pv" { "ridges_folded" } else { field };
                    if df_path(df_name).exists() || noise_path(field).exists() {
                        return Err(format!(
                            "v0 пак {} не должен менять {field}",
                            pack.display()
                        ));
                    }
                }
                for field in VANILLA_COMPUTED_FIELDS {
                    if df_path(field).exists() || noise_path(field).exists() {
                        return Err(format!(
                            "v0 пак {} не должен менять {field}",
                            pack.display()
                        ));
                    }
                }
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

fn version_arg(args: &[String]) -> Result<GenVersion, String> {
    match args.windows(2).find(|pair| pair[0] == "--version") {
        Some(pair) => {
            GenVersion::parse(&pair[1]).ok_or_else(|| format!("неизвестная версия {}", pair[1]))
        }
        None => Ok(GEN_VERSION),
    }
}

fn usage() -> &'static str {
    "xtask — задачи разработки\n\n\
     params-doc [--version V]      таблица параметров (v0 по умолчанию)\n\
     params-hash [--version V]     хэши параметров для scientific и gameplay\n\
     pack  [--out ПАПКА]           v0 + v1-dev: 2 режима каждый, и smoke\n\
     bundle [--out ПАПКА]          бандл нативного мода из шаблонов mod/\n\
     bundle-check [--out ПАПКА]    жёсткая проверка бандла (используется в CI)\n\
     help                          эта справка\n"
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("params-doc") => cmd_params_doc(version_arg(&args[1..])?),
        Some("params-hash") => cmd_params_hash(version_arg(&args[1..])?),
        Some("pack") => cmd_pack(&out_arg(&args[1..])?)?,
        Some("bundle") => cmd_bundle(&out_arg(&args[1..])?)?,
        Some("bundle-check") => cmd_bundle_check(&out_arg(&args[1..])?)?,
        Some("help") | Some("--help") | Some("-h") | None => print!("{text}", text = usage()),
        Some(other) => return Err(format!("неизвестная команда '{other}'")),
    }
    Ok(())
}
