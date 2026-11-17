//! Ф1 (docs/PLAN-FABRIC.md): C-интерфейс к `litho-core` для Java (Panama FFM).
//!
//! Правила:
//! - звать пакетами по колонкам, не по блоку;
//! - генератор потокобезопасный (`Generator: Send + Sync`), один указатель
//!   можно звать из многих потоков (Distant Horizons, C2ME);
//! - паника не пересекает границу FFI: ловится и даёт код ошибки / нули.

use std::ffi::c_char;
use std::panic::{AssertUnwindSafe, catch_unwind};

use litho_core::field::Field;
use litho_core::generator::Generator;
use litho_core::mode::Mode;
use litho_core::version::GenVersion;

/// Версия C-интерфейса. Java сверяет её при загрузке.
pub const ABI_VERSION: u32 = 1;

/// Число полей в пакетном ответе, порядок как `Field::ALL`:
/// continents, erosion, ridges, temperature, vegetation, lava, offset, pv, factor, jaggedness.
pub const FIELD_COUNT: usize = Field::ALL.len();

const CORE_VERSION: &[u8] = concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes();

#[unsafe(no_mangle)]
pub extern "C" fn litho_abi_version() -> u32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn litho_field_count() -> u32 {
    FIELD_COUNT as u32
}

/// Версия крейта, строка с нулём на конце; память статическая.
#[unsafe(no_mangle)]
pub extern "C" fn litho_core_version() -> *const c_char {
    CORE_VERSION.as_ptr().cast()
}

/// mode: 0 = scientific, 1 = gameplay. version: 0 = v0, 1 = v1, 2 = v1-dev.
/// Ноль (null) при ошибке.
#[unsafe(no_mangle)]
pub extern "C" fn litho_generator_new(seed: i64, mode: u32, version: u32) -> *mut Generator {
    let mode = match mode {
        0 => Mode::Scientific,
        1 => Mode::Gameplay,
        _ => return std::ptr::null_mut(),
    };
    let Some(&version) = GenVersion::ALL.get(version as usize) else {
        return std::ptr::null_mut();
    };
    match catch_unwind(|| Generator::new(seed, mode, version)) {
        Ok(Ok(generator)) => Box::into_raw(Box::new(generator)),
        _ => std::ptr::null_mut(),
    }
}

/// # Safety
/// `generator` получен из `litho_generator_new` и больше нигде не используется.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn litho_generator_free(generator: *mut Generator) {
    if !generator.is_null() {
        drop(unsafe { Box::from_raw(generator) });
    }
}

/// Пакет колонок: `xz` — `n` пар (x, z) в блоках; `out` — `n * FIELD_COUNT` f32,
/// по колонке подряд в порядке `Field::ALL`. Значения те же, что
/// `Generator::field_f32` (граница Horizon). Возвращает 0 или −1 при панике
/// (тогда `out` заполнен нулями).
///
/// # Safety
/// Указатели валидны на `2n` и `n * FIELD_COUNT` элементов.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn litho_fields_batch(
    generator: *const Generator,
    xz: *const f64,
    n: usize,
    out: *mut f32,
) -> i32 {
    if generator.is_null() || (n > 0 && (xz.is_null() || out.is_null())) {
        return -2;
    }
    let generator = unsafe { &*generator };
    let xz = unsafe { std::slice::from_raw_parts(xz, n * 2) };
    let out = unsafe { std::slice::from_raw_parts_mut(out, n * FIELD_COUNT) };
    let result = catch_unwind(AssertUnwindSafe(|| fill_fields(generator, xz, out)));
    if result.is_err() {
        out.fill(0.0);
        return -1;
    }
    0
}

pub fn fill_fields(generator: &Generator, xz: &[f64], out: &mut [f32]) {
    for (pair, row) in xz.chunks_exact(2).zip(out.chunks_exact_mut(FIELD_COUNT)) {
        for (slot, field) in row.iter_mut().zip(Field::ALL) {
            *slot = generator.field_f32(field, pair[0], 0.0, pair[1]);
        }
    }
}

/// Лава в ячейке аквифера (как хук Horizon). NaN при панике.
///
/// # Safety
/// `generator` валиден.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn litho_lava_aquifer(
    generator: *const Generator,
    cell_x: i64,
    cell_y: i64,
    cell_z: i64,
) -> f64 {
    if generator.is_null() {
        return f64::NAN;
    }
    let generator = unsafe { &*generator };
    catch_unwind(AssertUnwindSafe(|| {
        generator.lava_aquifer(cell_x, cell_y, cell_z)
    }))
    .unwrap_or(f64::NAN)
}

/// Глубинная лава у стыков: значение ячейки и флаг «суша глубоко».
/// Возвращает 0 или −1 при панике.
///
/// # Safety
/// `generator`, `value`, `deep_land` валидны.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn litho_lava_cell(
    generator: *const Generator,
    cell_x: i64,
    cell_z: i64,
    value: *mut f64,
    deep_land: *mut u8,
) -> i32 {
    if generator.is_null() || value.is_null() || deep_land.is_null() {
        return -2;
    }
    let generator = unsafe { &*generator };
    match catch_unwind(AssertUnwindSafe(|| {
        (
            generator.lava_cell(cell_x, cell_z),
            generator.lava_deep_land(cell_x, cell_z),
        )
    })) {
        Ok((v, d)) => {
            unsafe {
                *value = v;
                *deep_land = d as u8;
            }
            0
        }
        Err(_) => -1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn generator_is_thread_safe() {
        assert_send_sync::<Generator>();
    }

    #[test]
    fn batch_matches_core_bitwise() {
        let points: Vec<f64> = (0..64)
            .flat_map(|i| {
                let x = (i as f64 * 7919.0).rem_euclid(40_000.0) - 20_000.0;
                let z = (i as f64 * 104_729.0).rem_euclid(40_000.0) - 20_000.0;
                [x, z]
            })
            .collect();
        for (mode, version) in [(0u32, 2u32), (1, 2), (1, 0)] {
            let ffi = litho_generator_new(42, mode, version);
            assert!(!ffi.is_null());
            let core = Generator::new(
                42,
                Mode::ALL[mode as usize],
                GenVersion::ALL[version as usize],
            )
            .unwrap();
            let n = points.len() / 2;
            let mut out = vec![0f32; n * FIELD_COUNT];
            let code = unsafe { litho_fields_batch(ffi, points.as_ptr(), n, out.as_mut_ptr()) };
            assert_eq!(code, 0);
            for (i, pair) in points.chunks_exact(2).enumerate() {
                for (j, field) in Field::ALL.into_iter().enumerate() {
                    let want = core.field_f32(field, pair[0], 0.0, pair[1]);
                    assert_eq!(
                        out[i * FIELD_COUNT + j].to_bits(),
                        want.to_bits(),
                        "{field:?} {pair:?}"
                    );
                }
            }
            let mut v = 0.0;
            let mut d = 0u8;
            assert_eq!(unsafe { litho_lava_cell(ffi, 3, -5, &mut v, &mut d) }, 0);
            assert_eq!(v.to_bits(), core.lava_cell(3, -5).to_bits());
            assert_eq!(
                unsafe { litho_lava_aquifer(ffi, 1, -4, 2) }.to_bits(),
                core.lava_aquifer(1, -4, 2).to_bits()
            );
            unsafe { litho_generator_free(ffi) };
        }
    }

    #[test]
    fn bad_arguments_are_rejected() {
        assert!(litho_generator_new(1, 9, 0).is_null());
        assert!(litho_generator_new(1, 0, 9).is_null());
        assert_eq!(
            unsafe {
                litho_fields_batch(std::ptr::null(), std::ptr::null(), 0, std::ptr::null_mut())
            },
            -2
        );
    }
}
