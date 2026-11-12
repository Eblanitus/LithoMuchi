//! T151: подбор параметров без пересборки — `--set ключ=значение,…`.
//!
//! Только для песочницы. Замер с `--set` не является приёмкой: приёмка идёт
//! на `Params::defaults`, а найденные значения переносятся туда правкой кода.

use std::sync::OnceLock;

use litho_core::{GenVersion, Generator, Mode, Params};

static OVERRIDES: OnceLock<Vec<(String, f64)>> = OnceLock::new();

/// Разбирает `--set a=1,b[2]=3` один раз на процесс.
pub fn init(args: &[String]) -> Result<(), String> {
    let mut list = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        if arg != "--set" {
            continue;
        }
        let text = args.get(i + 1).ok_or("--set: нет значения")?;
        for item in text.split(',').filter(|item| !item.trim().is_empty()) {
            let (key, value) = item
                .split_once('=')
                .ok_or_else(|| format!("--set: ожидалось ключ=значение, получено «{item}»"))?;
            let value: f64 = value
                .trim()
                .parse()
                .map_err(|_| format!("--set {key}: «{value}» не число"))?;
            if !value.is_finite() {
                return Err(format!("--set {key}: значение не конечно"));
            }
            list.push((key.trim().to_string(), value));
        }
    }
    // Проверяем ключи сразу, на обоих режимах: опечатка не должна дать
    // «замер» на неизменённых параметрах.
    let mut probe = Params::defaults(GenVersion::V1Dev, Mode::Gameplay);
    for (key, value) in &list {
        set(&mut probe, key, *value)?;
    }
    OVERRIDES
        .set(list)
        .map_err(|_| "--set разобран дважды".to_string())
}

/// Текст для отчёта: пусто, если параметры не менялись.
pub fn label() -> String {
    OVERRIDES
        .get()
        .map(|list| {
            list.iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default()
}

pub fn params(mode: Mode, version: GenVersion) -> Result<Params, String> {
    let mut params = Params::defaults(version, mode);
    if let Some(list) = OVERRIDES.get() {
        if !list.is_empty() && !version.is_water_relief_dev() {
            return Err("--set разрешён только для v1-dev: v0 заморожена".into());
        }
        for (key, value) in list {
            set(&mut params, key, *value)?;
        }
    }
    Ok(params)
}

pub fn generator(seed: i64, mode: Mode, version: GenVersion) -> Result<Generator, String> {
    Generator::with_params(seed, params(mode, version)?).map_err(|error| error.to_string())
}

fn index(key: &str, text: &str, len: usize) -> Result<usize, String> {
    let i: usize = text
        .parse()
        .map_err(|_| format!("--set {key}: индекс «{text}» не число"))?;
    if i >= len {
        return Err(format!("--set {key}: индекс {i} вне 0…{}", len - 1));
    }
    Ok(i)
}

fn set(p: &mut Params, key: &str, value: f64) -> Result<(), String> {
    // `имя[i]` или `имя[i][j]`.
    let (name, indices) = match key.split_once('[') {
        Some((name, rest)) => {
            let rest = rest
                .strip_suffix(']')
                .ok_or_else(|| format!("--set {key}: нет закрывающей скобки"))?;
            (name, rest.split("][").collect::<Vec<_>>())
        }
        None => (key, Vec::new()),
    };
    let scalar: Option<&mut f64> = match name {
        "boundaries.ray_field_l" => Some(&mut p.boundaries.ray_field_l),
        "boundaries.ray_lattice_l" => Some(&mut p.boundaries.ray_lattice_l),
        "relief.a_lo" => Some(&mut p.relief.a_lo),
        "relief.a_hi" => Some(&mut p.relief.a_hi),
        "climate.m_e_lo" => Some(&mut p.climate.m_e_lo),
        "climate.m_e_hi" => Some(&mut p.climate.m_e_hi),
        "climate.e_mnt" => Some(&mut p.climate.e_mnt),
        "climate.e_mnt_noise" => Some(&mut p.climate.e_mnt_noise),
        "climate.e_plateau" => Some(&mut p.climate.e_plateau),
        "morpho.scientific_sigma_factor" => Some(&mut p.morpho.scientific_sigma_factor),
        "hydrology.floodplain_e" => Some(&mut p.hydrology.floodplain_e),
        "hydrology.dry_basin_e" => Some(&mut p.hydrology.dry_basin_e),
        "height.e_gain_base_km" => Some(&mut p.height.e_gain_base_km),
        "height.e_gain_blocks_per_km" => Some(&mut p.height.e_gain_blocks_per_km),
        _ => None,
    };
    if let Some(slot) = scalar {
        if !indices.is_empty() {
            return Err(format!("--set {key}: параметр не массив"));
        }
        *slot = value;
        return Ok(());
    }
    let row: Option<&mut [f64]> = match name {
        "morpho.e_flat" => Some(&mut p.morpho.e_flat),
        "morpho.e_hill" => Some(&mut p.morpho.e_hill),
        "morpho.sigma_e" => Some(&mut p.morpho.sigma_e),
        "morpho.sigma_pv" => Some(&mut p.morpho.sigma_pv),
        "height.micro_amp_blocks" => Some(&mut p.height.micro_amp_blocks),
        _ => None,
    };
    if let Some(row) = row {
        let [i] = indices.as_slice() else {
            return Err(format!("--set {key}: нужен один индекс, {name}[i]"));
        };
        let i = index(key, i, row.len())?;
        row[i] = value;
        return Ok(());
    }
    if name == "height.land_y" {
        let [i, j] = indices.as_slice() else {
            return Err(format!("--set {key}: нужны два индекса, {name}[i][j]"));
        };
        let i = index(key, i, p.height.land_y.len())?;
        let j = index(key, j, p.height.land_y[i].len())?;
        p.height.land_y[i][j] = value;
        return Ok(());
    }
    Err(format!(
        "--set: параметр «{name}» не входит в список подбора T151"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_changes_scalar_array_and_table() {
        let mut p = Params::defaults(GenVersion::V1Dev, Mode::Scientific);
        let before = p.params_hash();
        set(&mut p, "relief.a_hi", 0.7).unwrap();
        set(&mut p, "morpho.e_hill[3]", -0.4).unwrap();
        set(&mut p, "height.land_y[2][1]", 79.0).unwrap();
        assert_eq!(p.relief.a_hi, 0.7);
        assert_eq!(p.morpho.e_hill[3], -0.4);
        assert_eq!(p.height.land_y[2][1], 79.0);
        assert_ne!(p.params_hash(), before);
    }

    #[test]
    fn unknown_key_and_bad_index_are_errors() {
        let mut p = Params::defaults(GenVersion::V1Dev, Mode::Gameplay);
        assert!(set(&mut p, "relief.a_hii", 0.7).is_err());
        assert!(set(&mut p, "morpho.e_hill[8]", 0.0).is_err());
        assert!(set(&mut p, "morpho.e_hill", 0.0).is_err());
        assert!(set(&mut p, "relief.a_hi[0]", 0.0).is_err());
        assert!(set(&mut p, "height.land_y[1]", 0.0).is_err());
    }
}
