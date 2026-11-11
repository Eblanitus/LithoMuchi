//! T100/T144: предпросмотр высоты.
//!
//! Для v0 и явных --game-data/--synthetic-height это НЕ генератор чанков:
//! y = 128·(1 + offset) по локальным сплайнам, без factor/jaggedness/3D-шума.
//! В игре глубина на высоте y равна (1 − y/128) + offset, поэтому
//! поверхность лежит на y = 128·(1 + offset). Старая формула
//! 63,52 + 128·offset занижала высоту на 64,48 блока (К2, шаг 2).
//! Для v1-dev без явного источника используется собственная высота ядра
//! (HeightSource::Owned): сплайны Mojang больше не нужны. Файлы Mojang
//! читаются с диска пользователя, не копируются в репозиторий и не входят в
//! litho-core.

use std::path::{Path, PathBuf};

use litho_core::hash::fnv1a64;
use litho_core::vanilla::peaks_valleys;

use crate::json::{self, Json};

pub const SURFACE_BASE: f64 = 128.0;
pub const OFFSET_SCALE: f64 = 128.0;
pub const SEA_LEVEL: f64 = 63.0;

#[derive(Clone, Copy, Debug)]
pub struct HeightInput {
    pub c: f64,
    pub e: f64,
    pub w: f64,
    /// T141: по умолчанию PV = f(W); `--pv` подставляет собственное поле ядра.
    pub pv: f64,
    /// T144: готовая высота ядра для HeightSource::Owned. Сплайны её не
    /// пересчитывают; явный synthetic/game-data источник это поле игнорирует.
    pub owned_y: Option<f64>,
}
impl HeightInput {
    pub fn from_fields(c: f64, e: f64, w: f64) -> Self {
        Self {
            c,
            e,
            w,
            pv: peaks_valleys(w),
            owned_y: None,
        }
    }
    pub fn with_pv(mut self, pv: f64) -> Self {
        self.pv = pv;
        self
    }
    pub fn with_owned_y(mut self, y: f64) -> Self {
        self.owned_y = Some(y);
        self
    }
}

#[derive(Clone, Debug)]
pub enum HeightSource {
    GameData(PathBuf),
    /// Только явный флаг --synthetic-height: собственные тестовые числа.
    Synthetic,
    /// T144: собственная высота litho-core (v1-dev), без данных Mojang.
    Owned,
}
impl HeightSource {
    pub fn label(&self) -> String {
        match self {
            Self::GameData(path) => format!("local-game-data:{}", path.display()),
            Self::Synthetic => "synthetic-NOT-vanilla".into(),
            Self::Owned => "litho-core-owned-I2".into(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Input {
    C,
    E,
    W,
    Pv,
}

#[derive(Clone, Debug)]
enum Node {
    Constant(f64),
    Input(Input),
    Add(Box<Node>, Box<Node>),
    Mul(Box<Node>, Box<Node>),
    Lerp {
        alpha: Box<Node>,
        first: Box<Node>,
        second: Box<Node>,
    },
    Spline(Spline),
    /// Новая колонка/новый мир: смешивания со старыми чанками нет.
    NewChunkAlpha,
    OldChunkOffset,
}

#[derive(Clone, Debug)]
struct Spline {
    coordinate: Box<Node>,
    points: Vec<SplinePoint>,
}
#[derive(Clone, Debug)]
struct SplinePoint {
    location: f64,
    derivative: f64,
    value: Box<Node>,
}

impl Node {
    fn eval(&self, input: &HeightInput) -> Result<f64, String> {
        let value = match self {
            Self::Constant(v) => *v,
            Self::Input(field) => match field {
                Input::C => input.c,
                Input::E => input.e,
                Input::W => input.w,
                Input::Pv => input.pv,
            },
            Self::Add(a, b) => a.eval(input)? + b.eval(input)?,
            Self::Mul(a, b) => a.eval(input)? * b.eval(input)?,
            Self::Lerp {
                alpha,
                first,
                second,
            } => {
                let a = alpha.eval(input)?;
                // Не вычисляем старую ветвь blend_offset в новом мире.
                if a == 0.0 {
                    first.eval(input)?
                } else if a == 1.0 {
                    second.eval(input)?
                } else {
                    first.eval(input)? * (1.0 - a) + second.eval(input)? * a
                }
            }
            Self::Spline(spline) => spline.eval(input)?,
            Self::NewChunkAlpha => 1.0,
            Self::OldChunkOffset => return Err(
                "blend_offset доступен только для старых чанков; предпросмотр считает новый мир"
                    .into(),
            ),
        };
        if value.is_finite() {
            Ok(value)
        } else {
            Err("сплайн дал нечисловую высоту".into())
        }
    }
}

impl Spline {
    fn eval(&self, input: &HeightInput) -> Result<f64, String> {
        let x = self.coordinate.eval(input)?;
        if x <= self.points[0].location {
            return self.points[0].value.eval(input);
        }
        let last = self.points.len() - 1;
        if x >= self.points[last].location {
            return self.points[last].value.eval(input);
        }
        let right = self.points.partition_point(|p| p.location <= x);
        let left = right - 1;
        let a = &self.points[left];
        let b = &self.points[right];
        let av = a.value.eval(input)?;
        let bv = b.value.eval(input)?;
        let t = (x - a.location) / (b.location - a.location);
        // Cubic Hermite в форме ванильного CubicSpline: производные заданы
        // на единицу координаты, поэтому умножаются на расстояние до узла.
        let chord = av + (bv - av) * t;
        let tangent_a = av + a.derivative * (x - a.location);
        let tangent_b = bv - b.derivative * (b.location - x);
        let tangent = tangent_a + (tangent_b - tangent_a) * t;
        Ok(chord + t * (1.0 - t) * (tangent - chord))
    }
}

#[derive(Debug)]
pub struct HeightModel {
    pub source: HeightSource,
    pub data_hash: u64,
    offset: Node,
    factor: Node,
    jaggedness: Node,
}
impl HeightModel {
    pub fn load(source: HeightSource) -> Result<Self, String> {
        if matches!(source, HeightSource::Owned) {
            return Ok(Self {
                source,
                data_hash: fnv1a64("litho-core-owned-I2"),
                offset: Node::Constant(0.0),
                factor: Node::Constant(0.0),
                jaggedness: Node::Constant(0.0),
            });
        }
        let (texts, dir) = match &source {
            HeightSource::Synthetic => (
                [
                    SYNTHETIC_OFFSET.to_string(),
                    "0.0".to_string(),
                    "0.0".to_string(),
                ],
                None,
            ),
            HeightSource::Owned => unreachable!("owned обработан выше"),
            HeightSource::GameData(root) => {
                let dir = find_density_dir(root)?;
                let mut texts = Vec::with_capacity(3);
                for name in ["offset", "factor", "jaggedness"] {
                    let path = dir.join(format!("{name}.json"));
                    texts.push(
                        std::fs::read_to_string(&path)
                            .map_err(|e| format!("не читаю {}: {e}", path.display()))?,
                    );
                }
                let texts: [String; 3] = texts.try_into().map_err(|_| "ожидались три сплайна")?;
                (texts, Some(dir))
            }
        };
        let mut combined = String::new();
        let mut nodes = Vec::new();
        for (name, text) in ["offset", "factor", "jaggedness"].iter().zip(texts.iter()) {
            let label = match &dir {
                Some(dir) => dir.join(format!("{name}.json")).display().to_string(),
                None => format!("synthetic/{name}"),
            };
            let value = json::parse(text).map_err(|e| format!("{label}: {e}"))?;
            nodes.push(parse_node(&value).map_err(|e| format!("{label}: {e}"))?);
            combined.push_str(name);
            combined.push_str(text);
        }
        let mut nodes = nodes.into_iter();
        let model = Self {
            source,
            data_hash: fnv1a64(&combined),
            offset: nodes.next().expect("три сплайна"),
            factor: nodes.next().expect("три сплайна"),
            jaggedness: nodes.next().expect("три сплайна"),
        };
        // factor/jaggedness прочитаны и разобраны, но в формулу y не входят.
        // Проверяем, что их новая ветвь тоже вычислима, а не просто похожа на JSON.
        let zero = HeightInput {
            c: 0.0,
            e: 0.0,
            w: 0.0,
            pv: -1.0,
            owned_y: None,
        };
        model.factor.eval(&zero)?;
        model.jaggedness.eval(&zero)?;
        Ok(model)
    }
    pub fn surface_y(&self, input: HeightInput) -> Result<f64, String> {
        if self.is_owned() {
            return input
                .owned_y
                .filter(|y| y.is_finite())
                .ok_or_else(|| "owned-height: нет готовой высоты ядра".into());
        }
        Ok(SURFACE_BASE + OFFSET_SCALE * self.offset.eval(&input)?)
    }
    pub fn is_synthetic(&self) -> bool {
        matches!(self.source, HeightSource::Synthetic)
    }
    pub fn is_owned(&self) -> bool {
        matches!(self.source, HeightSource::Owned)
    }
}

fn find_density_dir(root: &Path) -> Result<PathBuf, String> {
    for candidate in [
        root.join("data/minecraft/worldgen/density_function/overworld"),
        root.join("minecraft/worldgen/density_function/overworld"),
        root.join("overworld"),
        root.to_path_buf(),
    ] {
        if candidate.join("offset.json").is_file() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "в {} нет data/minecraft/worldgen/density_function/overworld/offset.json; укажите распакованные данные игры",
        root.display()
    ))
}

fn field(value: &Json) -> Result<Node, String> {
    let name = value
        .as_str()
        .ok_or("координата/ссылка должна быть строкой")?;
    match name {
        "minecraft:overworld/continents" | "overworld/continents" => Ok(Node::Input(Input::C)),
        "minecraft:overworld/erosion" | "overworld/erosion" => Ok(Node::Input(Input::E)),
        "minecraft:overworld/ridges" | "overworld/ridges" => Ok(Node::Input(Input::W)),
        "minecraft:overworld/ridges_folded" | "overworld/ridges_folded" => {
            Ok(Node::Input(Input::Pv))
        }
        _ => Err(format!("неизвестная ссылка density_function '{name}'")),
    }
}
fn child(value: &Json, keys: &[&str]) -> Result<Box<Node>, String> {
    for key in keys {
        if let Some(child) = value.get(key) {
            return Ok(Box::new(parse_node(child)?));
        }
    }
    Err(format!("нет поля {}", keys.join("/")))
}
fn number(value: &Json) -> Result<f64, String> {
    value
        .as_f64()
        .filter(|v| v.is_finite())
        .ok_or_else(|| "ожидалось конечное число".into())
}
fn parse_node(value: &Json) -> Result<Node, String> {
    match value {
        Json::Number(v) if v.is_finite() => Ok(Node::Constant(*v)),
        Json::String(_) => field(value),
        Json::Object(_) => {
            let kind = value
                .get("type")
                .and_then(Json::as_str)
                .ok_or("у узла нет type")?;
            let kind = kind.strip_prefix("minecraft:").unwrap_or(kind);
            match kind {
                "constant" => Ok(Node::Constant(number(
                    value.get("argument").ok_or("constant без argument")?,
                )?)),
                "add" => Ok(Node::Add(
                    child(value, &["argument1", "left"])?,
                    child(value, &["argument2", "right"])?,
                )),
                "mul" => Ok(Node::Mul(
                    child(value, &["argument1", "left"])?,
                    child(value, &["argument2", "right"])?,
                )),
                "lerp" => Ok(Node::Lerp {
                    alpha: child(value, &["alpha"])?,
                    first: child(value, &["first"])?,
                    second: child(value, &["second"])?,
                }),
                "cache" | "cache_once" | "cache_2d" | "cache_all_in_cell" | "flat_cache"
                | "interpolated" => Ok(*child(value, &["argument", "input"])?),
                "blend_alpha" => Ok(Node::NewChunkAlpha),
                "blend_offset" => Ok(Node::OldChunkOffset),
                "spline" => Ok(Node::Spline(parse_spline(
                    value.get("spline").ok_or("spline без spline")?,
                )?)),
                _ => Err(format!(
                    "узел '{kind}' не поддержан предпросмотром; молчаливая подмена запрещена"
                )),
            }
        }
        _ => Err("узел должен быть числом, ссылкой или объектом".into()),
    }
}
fn parse_spline(value: &Json) -> Result<Spline, String> {
    if let Some(v) = value.as_f64() {
        return Ok(Spline {
            coordinate: Box::new(Node::Constant(0.0)),
            points: vec![
                SplinePoint {
                    location: 0.0,
                    derivative: 0.0,
                    value: Box::new(Node::Constant(v)),
                },
                SplinePoint {
                    location: 1.0,
                    derivative: 0.0,
                    value: Box::new(Node::Constant(v)),
                },
            ],
        });
    }
    let coordinate = Box::new(parse_node(
        value.get("coordinate").ok_or("сплайн без coordinate")?,
    )?);
    let items = value
        .get("points")
        .and_then(Json::as_array)
        .ok_or("сплайн без points")?;
    if items.len() < 2 {
        return Err("сплайн должен иметь хотя бы два узла".into());
    }
    let mut points = Vec::new();
    for item in items {
        let location = number(item.get("location").ok_or("узел без location")?)?;
        let derivative = number(item.get("derivative").ok_or("узел без derivative")?)?;
        if let Some(prev) = points.last().map(|p: &SplinePoint| p.location) {
            if location <= prev {
                return Err("location сплайна должны строго возрастать".into());
            }
        }
        let value = Box::new(parse_node(item.get("value").ok_or("узел без value")?)?);
        points.push(SplinePoint {
            location,
            derivative,
            value,
        });
    }
    Ok(Spline { coordinate, points })
}

/// Собственный маленький сплайн для тестов и демонстрации CLI. Это не данные
/// Mojang и не аппроксимация, выдаваемая за ваниль (явный флаг в метаданных).
const SYNTHETIC_OFFSET: &str = r#"{
  "type":"minecraft:add",
  "argument1":{"type":"minecraft:mul","argument1":0.25,"argument2":"minecraft:overworld/continents"},
  "argument2":{"type":"minecraft:add",
    "argument1":{"type":"minecraft:mul","argument1":-0.20,"argument2":"minecraft:overworld/erosion"},
    "argument2":{"type":"minecraft:mul","argument1":0.50,"argument2":"minecraft:overworld/ridges_folded"}
  }
}"#;

#[cfg(test)]
mod tests {
    use super::{HeightInput, HeightModel, HeightSource, Node, parse_node};
    use crate::json::parse;
    fn node(text: &str) -> Node {
        parse_node(&parse(text).unwrap()).unwrap()
    }
    #[test]
    fn synthetic_is_explicit_and_deterministic() {
        let a = HeightModel::load(HeightSource::Synthetic).unwrap();
        let b = HeightModel::load(HeightSource::Synthetic).unwrap();
        assert!(a.is_synthetic());
        assert_eq!(a.data_hash, b.data_hash);
        let input = HeightInput::from_fields(0.4, 0.2, 0.3);
        assert_eq!(
            a.surface_y(input).unwrap().to_bits(),
            b.surface_y(input).unwrap().to_bits()
        );
    }
    #[test]
    fn arithmetic_and_named_inputs() {
        let n = node(
            r#"{"type":"minecraft:mul","argument1":2,"argument2":{"type":"minecraft:add","argument1":"minecraft:overworld/continents","argument2":3}}"#,
        );
        assert_eq!(
            n.eval(&HeightInput::from_fields(4.0, 0.0, 0.0)).unwrap(),
            14.0
        );
    }
    #[test]
    fn pv_override_replaces_only_the_pv_input() {
        let input = HeightInput::from_fields(0.4, 0.2, 0.3);
        assert_eq!(input.pv, litho_core::vanilla::peaks_valleys(0.3));
        let overridden = input.with_pv(-1.0);
        assert_eq!(overridden.c, input.c);
        assert_eq!(overridden.e, input.e);
        assert_eq!(overridden.w, input.w);
        assert_eq!(overridden.pv, -1.0);
    }

    #[test]
    fn hermite_spline_uses_slopes_and_clamps_ends() {
        let n = node(
            r#"{"type":"minecraft:spline","spline":{"coordinate":"minecraft:overworld/continents","points":[{"location":0,"derivative":2,"value":1},{"location":1,"derivative":0,"value":3}]}}"#,
        );
        let at = |c| n.eval(&HeightInput::from_fields(c, 0.0, 0.0)).unwrap();
        assert_eq!(at(-1.0), 1.0);
        assert_eq!(at(2.0), 3.0);
        assert!((at(0.5) - 2.125).abs() < 1e-12);
    }
    #[test]
    fn new_chunk_branch_does_not_evaluate_old_blend_offset() {
        let n = node(
            r#"{"type":"minecraft:cache","input":{"type":"minecraft:lerp","alpha":{"type":"minecraft:blend_alpha"},"first":{"type":"minecraft:blend_offset"},"second":0.25}}"#,
        );
        assert_eq!(
            n.eval(&HeightInput::from_fields(0.0, 0.0, 0.0)).unwrap(),
            0.25
        );
    }
    #[test]
    fn rejects_unknown_nodes_and_bad_splines_instead_of_guessing() {
        for text in [
            r#"{"type":"minecraft:unknown"}"#,
            r#"{"type":"minecraft:spline","spline":{"coordinate":"minecraft:overworld/continents","points":[{"location":1,"derivative":0,"value":0},{"location":0,"derivative":0,"value":1}]}}"#,
            r#""minecraft:unknown/input""#,
        ] {
            assert!(parse_node(&parse(text).unwrap()).is_err());
        }
    }
    #[test]
    fn owned_height_uses_core_sample_and_ignores_splines() {
        let model = HeightModel::load(HeightSource::Owned).unwrap();
        assert!(model.is_owned());
        assert!(!model.is_synthetic());
        let input = HeightInput::from_fields(0.4, 0.2, 0.3).with_owned_y(123.0);
        assert_eq!(model.surface_y(input).unwrap(), 123.0);
        assert!(
            model
                .surface_y(HeightInput::from_fields(0.4, 0.2, 0.3))
                .is_err()
        );
        let synthetic = HeightModel::load(HeightSource::Synthetic).unwrap();
        assert!(synthetic.surface_y(input).unwrap().is_finite());
    }
    #[test]
    fn missing_game_data_is_actionable_error() {
        let error = HeightModel::load(HeightSource::GameData("/definitely/not/minecraft".into()))
            .unwrap_err();
        assert!(error.contains("offset.json"));
    }
}
