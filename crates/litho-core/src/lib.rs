//! # litho-core
//!
//! Ядро генерации мира Litospherity: вся «наука» без зависимостей от игры и без
//! внешних крейтов (план 6.1, 9.2). Оно отвечает на вопрос «какие значения
//! полей в точке `(x, z)` мира с сидом `S` в режиме `M` версии `V`».
//!
//! Статус: реализованы Э0, Э1a–Э1f, решётка и кэш (T13/T14), эталонный и
//! быстрый пути с пятью полями для игры (T55). Дальше: оценка автором (Э1h).

#![forbid(unsafe_code)]

pub mod cache;
pub mod classify;
pub mod climate;
pub mod crust;
pub mod drainage;
pub mod field;
pub mod generator;
pub mod hash;
pub mod lattice;
pub mod lava;
pub mod math;
pub mod mode;
pub mod noise;
pub mod noise_tables;
pub mod params;
pub mod plates;
pub mod rivers;
pub mod seed;
pub mod spawn;
pub mod vanilla;
pub mod version;

pub use classify::{ClassSample, LifeZone, VanillaCell};
pub use climate::{ClimateSample, PoleSample, PrecipSample, TemperatureSample};
pub use crust::CrustSample;
pub use field::{Field, FullSample, to_f32};
pub use generator::{GenError, Generator, WorldSample};
pub use math::Vec2;
pub use mode::{BlockParams, Mode};
pub use params::Params;
pub use plates::{BoundaryType, Plate, PlateSample};
pub use vanilla::VanillaSample;
pub use version::GenVersion;
