//! Отрисовка слоёв (план 12.2, 12.3).
//!
//! По умолчанию слои идут эталонным путём `Generator`; `--path fast`
//! переключает климатические слои на решётку и кэш (T55), чтобы картинки
//! песочницы совпадали с тем, что увидит игра.

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

/// Слои климата и выхода (12.3) считаются через `Generator::world_sample`.
pub const CLIMATE_LAYERS: [&str; 8] = [
    "poles", "temp", "wind", "moisture", "precip", "zones", "biomes", "vanilla",
];

pub const LAYERS: [&str; 20] = [
    "noise",
    "warp",
    "plates",
    "boundaries",
    "jacobian",
    "hash",
    "crust",
    "height",
    "mpi",
    "poles",
    "temp",
    "wind",
    "moisture",
    "precip",
    "zones",
    "biomes",
    "vanilla",
    "rivers",
    "lava",
    "spawn",
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
    pub path_fast: bool,
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

/// Слой `spawn` центрируется на найденной точке появления, остальные — на
/// `--center`. Сдвиг считается по сиду, поэтому одинаков во всех запусках.
pub fn options_for_layer(opts: &RenderOptions, layer: &str) -> RenderOptions {
    let mut out = opts.clone();
    if layer == "spawn" {
        if let Ok(generator) = Generator::new(opts.seed, opts.mode, opts.version) {
            let shift = generator.spawn_shift();
            out.center_x = shift.x * opts.mode.l_blocks();
            out.center_z = shift.z * opts.mode.l_blocks();
        }
    }
    out
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
        path = if opts.path_fast { "fast" } else { "reference" },
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
    let effective = options_for_layer(opts, layer);
    let noise_seed = subseed(generator.seed(), SALT_NOISE_DETAIL);
    let mut image = RgbImage::new(opts.size, opts.size);

    for py in 0..opts.size {
        for px in 0..opts.size {
            let (x, z) = effective.block_at_pixel(px, py);
            let p = generator.macro_point(x, z);
            let world = if CLIMATE_LAYERS.contains(&layer) {
                if opts.path_fast {
                    generator.world_sample_fast(x, z)
                } else {
                    generator.world_sample(x, z)
                }
            } else {
                None
            };
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
                "poles" => world
                    .as_ref()
                    .map_or([0, 0, 0], |w| palette::latitude(w.climate.pole.phi_deg)),
                "temp" => world
                    .as_ref()
                    .map_or([0, 0, 0], |w| palette::thermal(w.climate.temperature.t_bio)),
                "wind" => world.as_ref().map_or([0, 0, 0], |w| {
                    palette::wind_field(w.climate.wind, w.climate.wind.len())
                }),
                "moisture" => world.as_ref().map_or([0, 0, 0], |w| {
                    palette::moisture_field(w.climate.precip.mu, w.climate.precip.cont)
                }),
                "precip" => world.as_ref().map_or([0, 0, 0], |w| {
                    palette::precip_field(w.climate.precip.p_mm, w.climate.precip.ratio)
                }),
                "zones" => world.as_ref().map_or([0, 0, 0], |w| {
                    palette::zone_field(w.class.zone.belt, w.class.zone.province, w.class.is_ocean)
                }),
                "biomes" => world
                    .as_ref()
                    .map_or([0, 0, 0], |w| palette::biome_field(w.class.cell.biome)),
                "vanilla" => world.as_ref().map_or([0, 0, 0], |w| {
                    palette::vanilla_field(w.vanilla.c, w.vanilla.e, w.vanilla.w)
                }),
                "rivers" => match generator.crust_sample(x, z) {
                    Some(crust) => palette::river_field(generator.river_sample(x, z), crust.e_km),
                    None => [0, 0, 0],
                },
                "lava" => palette::heat(generator.lava_sample(x, z)),
                "spawn" => {
                    let quality = generator.spawn_quality(x, z);
                    let dx = x - effective.center_x;
                    let dz = z - effective.center_z;
                    let distance = (dx * dx + dz * dz).sqrt();
                    let ring =
                        (distance - params.spawn.search_radius_blocks).abs() < effective.bpp * 1.5;
                    palette::spawn_field(quality.score, ring)
                }
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
    let world_line = match generator.world_sample(x, z) {
        Some(w) => format!(
            "φ={phi}° T_bio={t_bio} °C T_min={t_min} °C |V|={wind} μ={mu} c={cont} P={p_mm} мм R={ratio}\n\
             зона=П{belt}/К{province} биом={biome} ({vt},{vh}) океан={ocean}\n\
             C={c} E={e} W={wv} T={vt_value} H={h}\n",
            phi = w.climate.pole.phi_deg,
            t_bio = w.climate.temperature.t_bio,
            t_min = w.climate.temperature.t_min,
            wind = w.climate.wind.len(),
            mu = w.climate.precip.mu,
            cont = w.climate.precip.cont,
            p_mm = w.climate.precip.p_mm,
            ratio = w.climate.precip.ratio,
            belt = w.class.zone.belt,
            province = w.class.zone.province,
            biome = w.class.cell.biome,
            vt = w.class.cell.t,
            vh = w.class.cell.h,
            ocean = w.is_ocean,
            c = w.vanilla.c,
            e = w.vanilla.e,
            wv = w.vanilla.w,
            vt_value = w.vanilla.t,
            h = w.vanilla.h,
        ),
        None => String::new(),
    };
    Ok(format!(
        "seed={seed} mode={mode} version={version} params={params_hash:016x}\n\
         блок ({x}, {z}) → макро p=({px}, {pz})\n\
         плита A id={id:#018x} ячейка=({ci}, {cj}) материк={continent} R={radius} age={age}\n\
         d_AB={d_ab} L, d_AC={d_ac} L, w_B={w_b}, w_C={w_c}\n\
         граница={bt:?}\n\
         якобиан искажения={jac}\n{crust_line}{world_line}",
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
        world_line = world_line,
    ))
}
