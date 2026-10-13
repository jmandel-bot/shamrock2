//! Just enough JSON to read back what this device keeps (Past games, its
//! tables, what happened) and write it again. No dependencies: values parse
//! to `Json`, object keys keep their order, and numbers are f64.

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
    pub fn parse(text: &str) -> Option<Json> {
        let mut p = Parser { s: text.as_bytes(), i: 0 };
        let v = p.value()?;
        p.ws();
        (p.i == p.s.len()).then_some(v)
    }

    pub fn get(&self, key: &str) -> &Json {
        match self {
            Json::Obj(kv) => kv.iter().find(|(k, _)| k == key).map_or(&Json::Null, |(_, v)| v),
            _ => &Json::Null,
        }
    }
    pub fn str(&self) -> Option<&str> {
        if let Json::Str(s) = self { Some(s) } else { None }
    }
    pub fn num(&self) -> Option<f64> {
        if let Json::Num(n) = self { Some(*n) } else { None }
    }
    pub fn bool(&self) -> Option<bool> {
        if let Json::Bool(b) = self { Some(*b) } else { None }
    }
    pub fn arr(&self) -> &[Json] {
        if let Json::Arr(a) = self { a } else { &[] }
    }
    /// Set (or add) `key` on an object.
    pub fn set(&mut self, key: &str, v: Json) {
        if let Json::Obj(kv) = self {
            match kv.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = v,
                None => kv.push((key.into(), v)),
            }
        }
    }
    pub fn str_of(s: &str) -> Json {
        Json::Str(s.into())
    }
}

impl std::fmt::Display for Json {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Json::Null => f.write_str("null"),
            Json::Bool(b) => write!(f, "{b}"),
            Json::Num(n) if n.fract() == 0.0 && n.abs() < 9e15 => write!(f, "{}", *n as i64),
            Json::Num(n) => write!(f, "{n}"),
            Json::Str(s) => f.write_str(&crate::model::json_str(s)),
            Json::Arr(a) => {
                f.write_str("[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{v}")?;
                }
                f.write_str("]")
            }
            Json::Obj(kv) => {
                f.write_str("{")?;
                for (i, (k, v)) in kv.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{}:{v}", crate::model::json_str(k))?;
                }
                f.write_str("}")
            }
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(|c| c.is_ascii_whitespace()) {
            self.i += 1;
        }
    }
    fn eat(&mut self, lit: &str) -> bool {
        let ok = self.s[self.i..].starts_with(lit.as_bytes());
        if ok {
            self.i += lit.len();
        }
        ok
    }
    fn value(&mut self) -> Option<Json> {
        self.ws();
        match *self.s.get(self.i)? {
            b'n' => self.eat("null").then_some(Json::Null),
            b't' => self.eat("true").then_some(Json::Bool(true)),
            b'f' => self.eat("false").then_some(Json::Bool(false)),
            b'"' => self.string().map(Json::Str),
            b'[' => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.eat("]") {
                    return Some(Json::Arr(a));
                }
                loop {
                    a.push(self.value()?);
                    self.ws();
                    if self.eat("]") {
                        return Some(Json::Arr(a));
                    }
                    if !self.eat(",") {
                        return None;
                    }
                }
            }
            b'{' => {
                self.i += 1;
                let mut kv = Vec::new();
                self.ws();
                if self.eat("}") {
                    return Some(Json::Obj(kv));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.ws();
                    if !self.eat(":") {
                        return None;
                    }
                    kv.push((k, self.value()?));
                    self.ws();
                    if self.eat("}") {
                        return Some(Json::Obj(kv));
                    }
                    if !self.eat(",") {
                        return None;
                    }
                }
            }
            _ => {
                let start = self.i;
                while self.s.get(self.i).is_some_and(|c| matches!(c, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')) {
                    self.i += 1;
                }
                std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok().map(Json::Num)
            }
        }
    }
    fn string(&mut self) -> Option<String> {
        if !self.eat("\"") {
            return None;
        }
        let mut out = Vec::new();
        loop {
            let c = *self.s.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => {
                    let e = *self.s.get(self.i)?;
                    self.i += 1;
                    let ch = match e {
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'u' => {
                            let hex = |p: &mut Self| -> Option<u32> {
                                let h = std::str::from_utf8(p.s.get(p.i..p.i + 4)?).ok()?;
                                p.i += 4;
                                u32::from_str_radix(h, 16).ok()
                            };
                            let mut u = hex(self)?;
                            // A surrogate pair (characters beyond the BMP, e.g. emoji).
                            if (0xd800..0xdc00).contains(&u) && self.eat("\\u") {
                                let lo = hex(self)?;
                                u = 0x10000 + ((u - 0xd800) << 10) + (lo.wrapping_sub(0xdc00) & 0x3ff);
                            }
                            char::from_u32(u).unwrap_or('\u{fffd}')
                        }
                        other => other as char,
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                _ => out.push(c),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Json;

    #[test]
    fn round_trips_what_the_page_stores() {
        let text = r#"[{"id":"1.2/3.4","room":"merry-otter-cove","round":2,"at":1760000000000,"players":["Ana","Bén 😀"],"boards":[{"text":"Ana\nsea\nsky\n\"q\"\n","guesses":1,"solved":true}],"finished":false}]"#;
        let v = Json::parse(text).unwrap();
        assert_eq!(v.arr()[0].get("players").arr()[1].str(), Some("Bén 😀"));
        assert_eq!(v.arr()[0].get("boards").arr()[0].get("text").str(), Some("Ana\nsea\nsky\n\"q\"\n"));
        assert_eq!(Json::parse(&v.to_string()), Some(v));
        assert_eq!(Json::parse(r#""😀""#), Some(Json::str_of("😀")));
        assert!(Json::parse("[1,").is_none() && Json::parse("{\"a\" 1}").is_none() && Json::parse("nul").is_none());
    }
}
