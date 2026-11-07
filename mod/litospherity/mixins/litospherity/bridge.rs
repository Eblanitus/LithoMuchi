// Litospherity bridge: клей между генератором мира (crates/litho-core) и Horizon.
// Знает типы Horizon; компилируется только внутри игры (план 5.4, T72).
//
// Эта модель по образцу distant_horizons/mixins/distant_horizons/bridge.rs.
// Все типы Horizon взяты из кода horizon-rust/horizon (ветка main, коммит 66f3981).

use horizon_core::noise::{DensityError, DensityVolume};
use horizon_core::position::BlockPos;

// --- Типы Horizon (заглушки, уточнить по сигнатурам версии 66f3981) ---

/// Игровой мир (заглушка — точная структура определена в horizon-client).
pub struct Game<'a> {
    pub seed: u64,
    pub _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> Game<'a> {
    pub fn seed(&self) -> u64 {
        self.seed
    }
}

/// Результат генерации чанка (заглушка).
pub struct ChunkResult {
    pub blocks: Vec<u8>,
    pub biome_ids: Vec<u16>,
}

/// Генератор мира Litospherity (мост к litho-core).
pub struct LitospherityGenerator {
    seed: u64,
}

impl LitospherityGenerator {
    pub fn new(seed: u64) -> Self {
        Self { seed }
    }

    /// Генерирует плотность блоков для чанка (x, z).
    /// Вызывается из хука native_fill через fill.rs.
    pub fn generate_chunk_density(
        &self,
        chunk_x: i32,
        chunk_z: i32,
    ) -> Result<Vec<f32>, DensityError> {
        // TODO: реальный вызов litho-core через FFI или встраиваемую библиотеку.
        //       Сейчас заглушка — возвращает объём воздуха.
        let volume = DensityVolume::new([16, 256, 16], [0, 0, 0], [1, 1, 1])?;
        let mut buffer = vec![0.0f32; volume.size()];
        Ok(buffer)
    }

    /// add_to_volume по колонкам (T74): одно значение на (x, z),
    /// прибавляется ко всем y с учётом amplitude.
    pub fn add_to_volume_columns(
        volume: &mut DensityVolume,
        x_min: i32,
        x_max: i32,
        z_min: i32,
        z_max: i32,
        y_min: i32,
        y_max: i32,
        amplitude: f64,
    ) -> Result<(), DensityError> {
        let amp = amplitude as f32;
        for z in z_min..=z_max {
            for x in x_min..=x_max {
                // TODO: реальный вызов field_column из litho-core.
                //       Сейчас заглушка — значение 0.0.
                let value = 0.0f32 * amp;
                for y in y_min..=y_max {
                    let idx = volume.index([(x - x_min) as usize, (z - z_min) as usize, (y - y_min) as usize]);
                    if let Some(i) = idx {
                        // unsafe или через getter/setter — уточнить по API DensityVolume.
                        let _ = (i, value);
                    }
                }
            }
        }
        Ok(())
    }
}
