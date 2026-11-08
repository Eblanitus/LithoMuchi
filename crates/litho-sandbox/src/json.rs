//! Маленький строгий JSON-ридер для локальных density_function (T100).
//! Только std: песочница не добавляет зависимостей в workspace Horizon.

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(map) => map.get(key),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Number(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(a) => Some(a),
            _ => None,
        }
    }
}

pub fn parse(text: &str) -> Result<Json, String> {
    if text.len() > 16 * 1024 * 1024 {
        return Err("JSON больше 16 МиБ".into());
    }
    let mut p = Parser {
        bytes: text.as_bytes(),
        pos: 0,
    };
    let value = p.value(0)?;
    p.whitespace();
    if p.pos != p.bytes.len() {
        return Err(p.error("лишние данные после JSON"));
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn error(&self, message: &str) -> String {
        format!("JSON, байт {}: {message}", self.pos)
    }
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
    fn take(&mut self, expected: u8) -> Result<(), String> {
        if self.peek() != Some(expected) {
            return Err(self.error(&format!("ожидалось '{}'", expected as char)));
        }
        self.pos += 1;
        Ok(())
    }
    fn whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.pos += 1;
        }
    }
    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > 128 {
            return Err(self.error("глубина JSON больше 128"));
        }
        self.whitespace();
        match self.peek() {
            Some(b'n') => self.literal("null", Json::Null),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b'[') => {
                self.pos += 1;
                let mut out = Vec::new();
                self.whitespace();
                if self.peek() == Some(b']') {
                    self.pos += 1;
                    return Ok(Json::Array(out));
                }
                loop {
                    out.push(self.value(depth + 1)?);
                    self.whitespace();
                    match self.peek() {
                        Some(b',') => self.pos += 1,
                        Some(b']') => {
                            self.pos += 1;
                            break;
                        }
                        _ => return Err(self.error("ожидалось ',' или ']'")),
                    }
                }
                Ok(Json::Array(out))
            }
            Some(b'{') => {
                self.pos += 1;
                let mut out = BTreeMap::new();
                self.whitespace();
                if self.peek() == Some(b'}') {
                    self.pos += 1;
                    return Ok(Json::Object(out));
                }
                loop {
                    self.whitespace();
                    let key = self.string()?;
                    self.whitespace();
                    self.take(b':')?;
                    let value = self.value(depth + 1)?;
                    if out.insert(key.clone(), value).is_some() {
                        return Err(self.error(&format!("повторный ключ '{key}'")));
                    }
                    self.whitespace();
                    match self.peek() {
                        Some(b',') => self.pos += 1,
                        Some(b'}') => {
                            self.pos += 1;
                            break;
                        }
                        _ => return Err(self.error("ожидалось ',' или '}'")),
                    }
                }
                Ok(Json::Object(out))
            }
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.error("ожидалось значение JSON")),
        }
    }
    fn literal(&mut self, text: &str, value: Json) -> Result<Json, String> {
        if self.bytes[self.pos..].starts_with(text.as_bytes()) {
            self.pos += text.len();
            Ok(value)
        } else {
            Err(self.error("неизвестный литерал"))
        }
    }
    fn number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return Err(self.error("неверное число")),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            let digits = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            if self.pos == digits {
                return Err(self.error("нет цифр после точки"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            let digits = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            if self.pos == digits {
                return Err(self.error("нет цифр в экспоненте"));
            }
        }
        // ASCII проверена выше; обрезка по границам символов невозможна.
        let text = std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|e| e.to_string())?;
        let number: f64 = text.parse().map_err(|_| self.error("неверное число"))?;
        if !number.is_finite() {
            return Err(self.error("число вне диапазона f64"));
        }
        Ok(Json::Number(number))
    }
    fn hex4(&mut self) -> Result<u32, String> {
        let mut value = 0_u32;
        for _ in 0..4 {
            let digit = match self.peek() {
                Some(c @ b'0'..=b'9') => u32::from(c - b'0'),
                Some(c @ b'a'..=b'f') => u32::from(c - b'a') + 10,
                Some(c @ b'A'..=b'F') => u32::from(c - b'A') + 10,
                _ => return Err(self.error("нужны четыре hex-цифры")),
            };
            self.pos += 1;
            value = value * 16 + digit;
        }
        Ok(value)
    }
    fn string(&mut self) -> Result<String, String> {
        self.take(b'"')?;
        let mut out = String::new();
        loop {
            let c = self.peek().ok_or_else(|| self.error("незакрытая строка"))?;
            self.pos += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let escape = self.peek().ok_or_else(|| self.error("незакрытый escape"))?;
                    self.pos += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut code = self.hex4()?;
                            if (0xD800..=0xDBFF).contains(&code) {
                                self.take(b'\\')?;
                                self.take(b'u')?;
                                let low = self.hex4()?;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return Err(self.error("неверная младшая surrogate-пара"));
                                }
                                code = 0x10000 + ((code - 0xD800) << 10) + low - 0xDC00;
                            } else if (0xDC00..=0xDFFF).contains(&code) {
                                return Err(self.error("одиночная младшая surrogate-пара"));
                            }
                            out.push(
                                char::from_u32(code)
                                    .ok_or_else(|| self.error("неверный Unicode"))?,
                            );
                        }
                        _ => return Err(self.error("неверный escape")),
                    }
                }
                0..=0x1F => return Err(self.error("управляющий символ в строке")),
                _ => {
                    // Копируем UTF-8 целиком, а не байт за байтом.
                    let start = self.pos - 1;
                    while self.peek().is_some_and(|b| b & 0xC0 == 0x80) {
                        self.pos += 1;
                    }
                    let piece = std::str::from_utf8(&self.bytes[start..self.pos])
                        .map_err(|_| self.error("неверный UTF-8"))?;
                    out.push_str(piece);
                }
            }
        }
    }
}

/// Экранирование строки для собственных JSON-подписей песочницы.
pub fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::{Json, parse, quote};
    #[test]
    fn parses_nested_json_and_unicode() {
        let value =
            parse(r#"{"a":[1,-2.5e2,true,null,"\u043f\u0440\ud83d\ude00"],"b":{}}"#).unwrap();
        assert_eq!(
            value.get("a").unwrap().as_array().unwrap()[1].as_f64(),
            Some(-250.0)
        );
        assert_eq!(
            value.get("a").unwrap().as_array().unwrap()[4].as_str(),
            Some("пр😀")
        );
        assert_eq!(value.get("b"), Some(&Json::Object(Default::default())));
    }
    #[test]
    fn rejects_malformed_and_ambiguous_json() {
        for text in [
            "",
            "{",
            "{\"a\":1,\"a\":2}",
            "[1,]",
            "01",
            "1e999",
            "\"\\uD800\"",
            "\"x\ny\"",
            "{} false",
        ] {
            assert!(parse(text).is_err(), "принято {text:?}");
        }
        let deep = format!("{}0{}", "[".repeat(130), "]".repeat(130));
        assert!(parse(&deep).is_err());
    }
    #[test]
    fn quotes_control_characters() {
        assert_eq!(quote("a\"\\\n\u{1}"), "\"a\\\"\\\\\\n\\u0001\"");
    }
}
