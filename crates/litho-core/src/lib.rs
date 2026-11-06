//! # litho-core
//!
//! Ядро генерации мира Litospherity: вся «наука» без зависимостей от игры и без
//! внешних крейтов (план 6.1, 9.2). Оно отвечает на вопрос «какие значения
//! полей в точке `(x, z)` мира с сидом `S` в режиме `M` версии `V`».
//!
//! Статус: реализованы Э0, Э1a и первая часть Э1b — хэши, шумы, параметры,
//! литосферные плиты и типы границ. Дальше по плану: кора и рельеф (T22–T24),
//! климат (T30–T33), классификация (T40–T43), решётка и `FullSample` (T13, T55).

#![forbid(unsafe_code)]

pub mod crust;
pub mod generator;
pub mod hash;
pub mod math;
pub mod mode;
pub mod noise;
pub mod noise_tables;
pub mod params;
pub mod plates;
pub mod seed;
pub mod version;

pub use crust::CrustSample;
pub use generator::{GenError, Generator};
pub use math::Vec2;
pub use mode::{BlockParams, Mode};
pub use params::Params;
pub use plates::{BoundaryType, Plate, PlateSample};
pub use version::GenVersion;
