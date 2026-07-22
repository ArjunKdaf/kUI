//! Minimal, tolerant JSON value parser — just enough for GitHub API
//! responses and the storefront manifest. Hand-rolled, std only.

/// A parsed JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    /// Parse a JSON document from a string.
    pub fn parse(s: &str) -> Result<Json, String> {
        let bytes = s.as_bytes();
        let mut p = Parser { b: bytes, i: 0 };
        p.skip_ws();
        let v = p.value()?;
        p.skip_ws();
        // Tolerate trailing garbage (some servers append newlines etc.).
        Ok(v)
    }

    /// Object field lookup.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Array index lookup.
    pub fn idx(&self, i: usize) -> Option<&Json> {
        match self {
            Json::Arr(items) => items.get(i),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(items) => Some(items),
            _ => None,
        }
    }
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn err(&self, msg: &str) -> String {
        format!("json parse error at byte {}: {}", self.i, msg)
    }

    fn value(&mut self) -> Result<Json, String> {
        self.skip_ws();
        match self.peek() {
            None => Err(self.err("unexpected end of input")),
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(c) if c == b'-' || c.is_ascii_digit() => self.number(),
            Some(c) => Err(self.err(&format!("unexpected character '{}'", c as char))),
        }
    }

    fn literal(&mut self, word: &str, v: Json) -> Result<Json, String> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(self.err(&format!("expected '{word}'")))
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-'))
        {
            self.i += 1;
        }
        let text = std::str::from_utf8(&self.b[start..self.i]).map_err(|_| self.err("bad number"))?;
        text.parse::<f64>()
            .map(Json::Num)
            .map_err(|_| self.err(&format!("bad number '{text}'")))
    }

    fn string(&mut self) -> Result<String, String> {
        // caller guarantees a leading '"'
        self.i += 1;
        let mut out = String::new();
        loop {
            let c = self.peek().ok_or_else(|| self.err("unterminated string"))?;
            self.i += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let e = self.peek().ok_or_else(|| self.err("unterminated escape"))?;
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let ch = if (0xD800..0xDC00).contains(&hi) {
                                // surrogate pair: expect \uXXXX low surrogate
                                if self.b[self.i..].starts_with(b"\\u") {
                                    self.i += 2;
                                    let lo = self.hex4()?;
                                    if (0xDC00..0xE000).contains(&lo) {
                                        let cp =
                                            0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                                        char::from_u32(cp)
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            } else {
                                char::from_u32(hi)
                            };
                            // tolerant: bad surrogates become the replacement char
                            out.push(ch.unwrap_or('\u{FFFD}'));
                        }
                        other => {
                            // tolerant: keep unknown escapes verbatim
                            out.push('\\');
                            out.push(other as char);
                        }
                    }
                }
                _ => {
                    // Collect the full UTF-8 sequence starting at c.
                    let len = utf8_len(c);
                    let start = self.i - 1;
                    let end = (start + len).min(self.b.len());
                    self.i = end;
                    match std::str::from_utf8(&self.b[start..end]) {
                        Ok(s) => out.push_str(s),
                        Err(_) => out.push('\u{FFFD}'),
                    }
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        if self.i + 4 > self.b.len() {
            return Err(self.err("truncated \\u escape"));
        }
        let s = std::str::from_utf8(&self.b[self.i..self.i + 4])
            .map_err(|_| self.err("bad \\u escape"))?;
        let v = u32::from_str_radix(s, 16).map_err(|_| self.err("bad \\u escape"))?;
        self.i += 4;
        Ok(v)
    }

    fn array(&mut self) -> Result<Json, String> {
        self.i += 1; // '['
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b']') => {
                    self.i += 1;
                    return Ok(Json::Arr(items));
                }
                None => return Err(self.err("unterminated array")),
                _ => {}
            }
            items.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.i += 1; // tolerant: allows trailing comma
                }
                Some(b']') => {}
                _ => return Err(self.err("expected ',' or ']' in array")),
            }
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.i += 1; // '{'
        let mut fields = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Json::Obj(fields));
                }
                Some(b'"') => {}
                None => return Err(self.err("unterminated object")),
                _ => return Err(self.err("expected '\"' or '}' in object")),
            }
            let key = self.string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(self.err("expected ':' after object key"));
            }
            self.i += 1;
            let val = self.value()?;
            fields.push((key, val));
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.i += 1; // tolerant: allows trailing comma
                }
                Some(b'}') => {}
                _ => return Err(self.err("expected ',' or '}' in object")),
            }
        }
    }
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars() {
        assert_eq!(Json::parse("null").unwrap(), Json::Null);
        assert_eq!(Json::parse(" true ").unwrap(), Json::Bool(true));
        assert_eq!(Json::parse("false").unwrap(), Json::Bool(false));
        assert_eq!(Json::parse("42").unwrap().as_f64(), Some(42.0));
        assert_eq!(Json::parse("-3.5e2").unwrap().as_f64(), Some(-350.0));
        assert_eq!(Json::parse("\"hi\"").unwrap().as_str(), Some("hi"));
    }

    #[test]
    fn escapes() {
        let v = Json::parse(r#""a\nb\t\"q\"\\Aé""#).unwrap();
        assert_eq!(v.as_str(), Some("a\nb\t\"q\"\\A\u{e9}"));
        // surrogate pair (emoji)
        let v = Json::parse(r#""😀""#).unwrap();
        assert_eq!(v.as_str(), Some("\u{1F600}"));
        // raw multi-byte UTF-8 passes through
        let v = Json::parse("\"caf\u{e9} \u{1F3AE}\"").unwrap();
        assert_eq!(v.as_str(), Some("caf\u{e9} \u{1F3AE}"));
    }

    #[test]
    fn nested_structures() {
        let src = r#"
        {
            "name": "kUI PakDek",
            "paks": [
                { "id": "a1", "version": "v1.0.0", "disabled": false,
                  "platforms": ["tg5040", "tg5050"], "large_pak": false },
                { "id": "b2", "disabled": true, "platforms": [] }
            ],
            "count": 2,
            "meta": null
        }"#;
        let v = Json::parse(src).unwrap();
        assert_eq!(v.get("name").and_then(Json::as_str), Some("kUI PakDek"));
        assert_eq!(v.get("count").and_then(Json::as_f64), Some(2.0));
        assert_eq!(v.get("meta"), Some(&Json::Null));
        let paks = v.get("paks").unwrap();
        assert_eq!(paks.as_arr().unwrap().len(), 2);
        let first = paks.idx(0).unwrap();
        assert_eq!(first.get("id").and_then(Json::as_str), Some("a1"));
        assert_eq!(first.get("disabled").and_then(Json::as_bool), Some(false));
        assert_eq!(
            first
                .get("platforms")
                .and_then(|p| p.idx(0))
                .and_then(Json::as_str),
            Some("tg5040")
        );
        assert_eq!(paks.idx(1).unwrap().get("disabled").and_then(Json::as_bool), Some(true));
        // misses
        assert!(v.get("nope").is_none());
        assert!(paks.idx(9).is_none());
        assert!(v.idx(0).is_none()); // not an array
    }

    #[test]
    fn tolerant_bits() {
        // trailing commas and surrounding whitespace are accepted
        let v = Json::parse("{ \"a\": [1, 2,], }\n").unwrap();
        assert_eq!(v.get("a").unwrap().as_arr().unwrap().len(), 2);
    }

    #[test]
    fn rejects_garbage() {
        assert!(Json::parse("").is_err());
        assert!(Json::parse("{\"a\" 1}").is_err());
        assert!(Json::parse("[1, 2").is_err());
        assert!(Json::parse("\"unterminated").is_err());
    }
}
