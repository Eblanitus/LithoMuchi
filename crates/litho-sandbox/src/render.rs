//! Отрисовка слоёв (план 12.2, 12.3).
//!
//! Пока все слои идут эталонным путём `Generator` без решётки и кэша: они и
//! должны совпадать с будущим быстрым путём с точностью допуска 14.2, но сетка
//! ещё не реализована (T13/T55). Поэтому флаг `--path` принят, но оба значения
//! сейчас дают эталонный путь.

use litho_core::Generator;
use litho_core::hash::hash2;
use litho_core::math::{Vec2, smoothstep};
use litho_core::mode::Mode;
use litho_core::noise::fbm2;
use litho_core::params::Params;
use litho_core::plates::{boundary_type, plate_color, warped_point};
use litho_core::seed::{SALT_NOISE_DETAIL, subseed};
use litho_core::version::GenVersion;

use crate::palette;
use crate::png::RgbImage;

pub const LAYERS: [&str; 9] = [
    "noise",
    "warp",
    "plates",
    "boundaries",
    "jacobian",
    "hash",
    "crust",
    "height",
    "mpi",
];

#[derive(Clone, Debug)]
pub struct RenderOptions {
    pub seed: i64,
    pub mode: Mode,
    pub version: GenVersion,
    pub center_x: f64,
    pub center_z: f64,
    pub size: usize,
    pub bpp: f64,
}

impl RenderOptions {
    pub fn block_at_pixel(&self, px: usize, py: usize) -> (f64, f64) {
        let half = self.size as f64 * 0.5;
        (
            self.center_x + (px as f64 - half) * self.bpp,
            self.center_z + (py as f64 - half) * self.bpp,
        )
    }
}

/// Подпись картинки (12.3): сид, режим, версия, `params_hash`, масштаб, слой.
/// Лежит рядом с PNG в отдельном `.json`, чтобы не тащить в песочницу шрифты.
pub fn metadata(opts: &RenderOptions, layer: &str, params_hash: u64) -> String {
    format!(
        "{{\n  \"layer\": \"{layer}\",\n  \"seed\": {seed},\n  \"mode\": \"{mode}\",\n  \"version\": \"{version}\",\n  \"params_hash\": \"{params_hash:016x}\",\n  \"center\": [{cx}, {cz}],\n  \"size\": {size},\n  \"blocks_per_pixel\": {bpp},\n  \"path\": \"reference\"\n}}\n",
        seed = opts.seed,
        mode = opts.mode.as_str(),
        version = opts.version.as_str(),
        cx = opts.center_x,
        cz = opts.center_z,
        size = opts.size,
        bpp = opts.bpp,
    )
}

pub fn render_layer(opts: &RenderOptions, layer: &str) -> Result<RgbImage, String> {
    if !LAYERS.contains(&layer) {
        let available = LAYERS.join(", ");
        return Err(format!("неизвестный слой '{layer}'; доступно: {available}"));
    }
    let generator =
        Generator::new(opts.seed, opts.mode, opts.version).map_err(|error| error.to_string())?;
    let params: &Params = generator.params();
    let noise_seed = subseed(generator.seed(), SALT_NOISE_DETAIL);
    let mut image = RgbImage::new(opts.size, opts.size);

    for py in 0..opts.size {
        for px in 0..opts.size {
            let (x, z) = opts.block_at_pixel(px, py);
            let p = generator.macro_point(x, z);
            let rgb = match layer {
                "noise" => {
                    let n = fbm2(noise_seed, p.x, p.z, 24.0, 4, 0.45);
                    palette::noise(n)
                }
                "warp" => {
                    let q = warped_point(p, generator.seed(), params);
                    let shift = q.sub(p).len() / params.plates.warp_amplitude_l;
                    palette::heat(shift)
                }
                "plates" => plates_color(&generator, x, z, params, false),
                "boundaries" => plates_color(&generator, x, z, params, true),
                "jacobian" => palette::jacobian(generator.jacobian(x, z)),
                "crust" => match generator.crust_sample(x, z) {
                    Some(s) => palette::kappa(s.kappa_l),
                    None => [0, 0, 0],
                },
                "height" => match generator.crust_sample(x, z) {
                    Some(s) => palette::hypsometric(s.e_km),
                    None => [0, 0, 0],
                },
                "mpi" => match generator.crust_sample(x, z) {
                    Some(s) => palette::mpi(s.mountain, s.plateau, s.iota_l),
                    None => [0, 0, 0],
                },
                "hash" => {
                    let i = (p.x / 4.0).floor() as i64;
                    let j = (p.z / 4.0).floor() as i64;
                    palette::from_hash(hash2(noise_seed, i, j))
                }
                _ => unreachable!("список слоёв проверен выше"),
            };
            image.set(px, py, rgb);
        }
    }
    Ok(image)
}

/// Цвет плиты с наложенным цветом границы. В режиме `boundaries_only` фон
/// чёрный, чтобы были видны только стыки.
fn plates_color(
    generator: &Generator,
    x: f64,
    z: f64,
    params: &Params,
    boundaries_only: bool,
) -> [u8; 3] {
    let Some(sample) = generator.plate_sample(x, z) else {
        return [0, 0, 0];
    };
    let base = if boundaries_only {
        [12, 12, 18]
    } else {
        plate_color(sample.plate_id())
    };
    let Some(b) = sample.b else {
        return base;
    };
    let bt = boundary_type(&sample.a, &b, sample.q, sample.n_ab, params);
    let (color, weight) = if bt.conv >= bt.div && bt.conv >= bt.tr {
        (palette::CONVERGENT, bt.conv)
    } else if bt.div >= bt.tr {
        (palette::DIVERGENT, bt.div)
    } else {
        (palette::TRANSFORM, bt.tr)
    };
    let near = smoothstep(0.4, 0.0, sample.d_ab_l);
    let shade = if boundaries_only { 1.0 } else { 0.9 };
    palette::mix(base, color, near * weight.max(0.15) * shade)
}

/// Текстовая проба точки для команды `probe` (12.2).
pub fn probe_text(opts: &RenderOptions, x: f64, z: f64) -> Result<String, String> {
    let generator =
        Generator::new(opts.seed, opts.mode, opts.version).map_err(|error| error.to_string())?;
    let params = generator.params();
    let p: Vec2 = generator.macro_point(x, z);
    let Some(sample) = generator.plate_sample(x, z) else {
        return Ok(format!("точка ({x}, {z}): плита не найдена"));
    };
    let bt = sample
        .b
        .map(|b| boundary_type(&sample.a, &b, sample.q, sample.n_ab, params));
    let crust_line = match generator.crust_sample(x, z) {
        Some(cs) => format!(
            "κ={kappa} L, e={e} км, M={m}, Pl={pl}, ι={iota} L, k={k:?}\n",
            kappa = cs.kappa_l,
            e = cs.e_km,
            m = cs.mountain,
            pl = cs.plateau,
            iota = cs.iota_l,
            k = cs.boundary_weights,
        ),
        None => String::new(),
    };
    Ok(format!(
        "seed={seed} mode={mode} version={version} params={params_hash:016x}\n\
         блок ({x}, {z}) → макро p=({px}, {pz})\n\
         плита A id={id:#018x} ячейка=({ci}, {cj}) материк={continent} R={radius} age={age}\n\
         d_AB={d_ab} L, d_AC={d_ac} L, w_B={w_b}, w_C={w_c}\n\
         граница={bt:?}\n\
         якобиан искажения={jac}\n{crust_line}",
        seed = opts.seed,
        mode = opts.mode.as_str(),
        version = opts.version.as_str(),
        params_hash = params.params_hash(),
        px = p.x,
        pz = p.z,
        id = sample.a.id,
        ci = sample.a.cell_i,
        cj = sample.a.cell_j,
        continent = sample.a.has_continent,
        radius = sample.a.radius_l,
        age = sample.a.age,
        d_ab = sample.d_ab_l,
        d_ac = sample.d_ac_l,
        w_b = sample.w_b,
        w_c = sample.w_c,
        bt = bt,
        jac = generator.jacobian(x, z),
        crust_line = crust_line,
    ))
}
