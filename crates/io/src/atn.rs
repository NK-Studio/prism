//! Bounded Photoshop Actions (`.atn`, version 16) reader.
//!
//! Independently implemented from Adobe's public File Formats specification and
//! read-only observations of installed action files. Parsing does not run commands.
//! Unlike PSD descriptors, ATN identifier/index references include a class header.
//! The original descriptor bytes are retained for commands Prism cannot translate.

pub use prism_psd::descriptor::{Class, Id, UnicodeString};

type Result<T> = std::result::Result<T, String>;
/// Maximum accepted file size, before parsing or copying any payload.
pub const MAX_FILE_BYTES: usize = 64 << 20;
const MAX_NODES: usize = 200_000;
const MAX_DEPTH: usize = 32;
const MAX_STRING_UNITS: usize = 1 << 20;

/// A set and its recorded actions, in source order.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionSet {
    pub name: String,
    pub expanded: bool,
    pub actions: Vec<Action>,
}

/// A recorded action. Shortcut and color metadata are preserved, not activated.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub name: String,
    pub index: u16,
    pub shift_key: bool,
    pub command_key: bool,
    pub color: u16,
    pub expanded: bool,
    pub steps: Vec<Step>,
}

/// An event plus its playback flags and typed parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub expanded: bool,
    pub enabled: bool,
    pub dialog: bool,
    pub dialog_options: u8,
    pub event: Id,
    pub name: String,
    /// Dictionary-name bytes, preserved even when not UTF-8.
    pub name_bytes: Vec<u8>,
    pub descriptor: Option<Descriptor>,
    /// Exact unversioned descriptor bytes; empty when no descriptor is present.
    pub descriptor_bytes: Vec<u8>,
}

/// An ordered descriptor with unmodified source identifiers.
#[derive(Debug, Clone, PartialEq)]
pub struct Descriptor {
    pub name: UnicodeString,
    pub class_id: Id,
    pub items: Vec<(Id, Value)>,
}
impl Descriptor {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.items.iter().find(|(id, _)| id.is(key)).map(|(_, value)| value)
    }
}

/// ATN reference forms; every form carries a class, including index/identifier.
#[derive(Debug, Clone, PartialEq)]
pub enum ReferenceItem {
    Property { class: Class, key: Id },
    Class(Class),
    Enumerated { class: Class, type_id: Id, value: Id },
    Offset { class: Class, offset: i32 },
    Identifier { class: Class, value: u32 },
    Index { class: Class, value: u32 },
    Name { class: Class, name: UnicodeString },
}

/// Typed descriptor values. Opaque paths/aliases/data are retained, never opened.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Reference(Vec<ReferenceItem>),
    Descriptor(Descriptor),
    GlobalObject(Descriptor),
    List(Vec<Value>),
    Double(f64),
    UnitFloat { unit: [u8; 4], value: f64 },
    UnitFloats { unit: [u8; 4], values: Vec<f64> },
    Text(UnicodeString),
    Enumerated { type_id: Id, value: Id },
    Integer(i32),
    LargeInteger(i64),
    Boolean(bool),
    Class(Class),
    GlobalClass(Class),
    Alias(Vec<u8>),
    Path(Vec<u8>),
    RawData(Vec<u8>),
    ObjectArray { prefix: u32, body: Descriptor },
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    nodes: usize,
    ctl: &'a prism_raster::Interrupt<'a>,
}
impl<'a> Reader<'a> {
    fn error(&self, message: &str) -> String {
        format!("Photoshop ATN at byte {}: {message}", self.pos)
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or_else(|| self.error("length overflow"))?;
        let bytes = self.bytes.get(self.pos..end).ok_or_else(|| self.error("truncated data"))?;
        self.pos = end;
        Ok(bytes)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?.try_into().map_err(|_| self.error("truncated number"))
    }
    fn byte(&mut self) -> Result<u8> {
        self.take(1)?.first().copied().ok_or_else(|| self.error("truncated byte"))
    }
    fn boolean(&mut self) -> Result<bool> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(self.error("invalid boolean")),
        }
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.array()?))
    }
    fn double(&mut self) -> Result<f64> {
        Ok(f64::from_be_bytes(self.array()?))
    }
    fn count(&mut self, cap: usize, minimum_bytes: usize) -> Result<usize> {
        let count = usize::try_from(self.u32()?).map_err(|_| self.error("count overflow"))?;
        let remaining = self.bytes.len().saturating_sub(self.pos);
        if count > cap || count > remaining / minimum_bytes.max(1) {
            return Err(self.error("count exceeds available data or limit"));
        }
        Ok(count)
    }
    fn node(&mut self, depth: usize) -> Result<()> {
        self.ctl.check().map_err(|e| e.to_string())?;
        self.nodes = self.nodes.saturating_add(1);
        if depth > MAX_DEPTH || self.nodes > MAX_NODES {
            return Err(self.error("nesting or total item limit exceeded"));
        }
        Ok(())
    }
    fn unicode(&mut self) -> Result<UnicodeString> {
        let count = self.count(MAX_STRING_UNITS, 2)?;
        let bytes = self.take(count.saturating_mul(2))?;
        let units = bytes
            .chunks_exact(2)
            .map(|pair| {
                let mut it = pair.iter().copied();
                u16::from_be_bytes([it.next().unwrap_or(0), it.next().unwrap_or(0)])
            })
            .collect();
        Ok(UnicodeString(units))
    }
    fn blob(&mut self) -> Result<Vec<u8>> {
        let count = self.count(MAX_FILE_BYTES, 1)?;
        Ok(self.take(count)?.to_vec())
    }
    fn id(&mut self) -> Result<Id> {
        let count = self.count(MAX_STRING_UNITS, 1)?;
        if count == 0 { Ok(Id::Code(self.array()?)) } else { Ok(Id::Str(self.take(count)?.to_vec())) }
    }
    fn class(&mut self) -> Result<Class> {
        Ok(Class { name: self.unicode()?, class_id: self.id()? })
    }
    fn descriptor(&mut self, depth: usize) -> Result<Descriptor> {
        self.node(depth)?;
        let name = self.unicode()?;
        let class_id = self.id()?;
        let count = self.count(MAX_NODES, 10)?;
        let mut items = Vec::new();
        let mut keys = std::collections::HashSet::new();
        for _ in 0..count {
            let id = self.id()?;
            // Duplicate keys are ambiguous to a command mapper; refuse rather than
            // let its first/last-key lookup silently change the recorded operation.
            if !keys.insert(id.as_bytes().to_vec()) {
                return Err(self.error("duplicate descriptor key"));
            }
            items.push((id, self.value(depth + 1)?));
        }
        Ok(Descriptor { name, class_id, items })
    }
    fn reference(&mut self, depth: usize) -> Result<ReferenceItem> {
        self.node(depth)?;
        let tag: [u8; 4] = self.array()?;
        let class = self.class()?;
        Ok(match &tag {
            b"prop" => ReferenceItem::Property { class, key: self.id()? },
            b"Clss" => ReferenceItem::Class(class),
            b"Enmr" => ReferenceItem::Enumerated { class, type_id: self.id()?, value: self.id()? },
            b"rele" => ReferenceItem::Offset { class, offset: self.i32()? },
            b"Idnt" => ReferenceItem::Identifier { class, value: self.u32()? },
            b"indx" => ReferenceItem::Index { class, value: self.u32()? },
            b"name" => ReferenceItem::Name { class, name: self.unicode()? },
            _ => return Err(self.error(&format!("unsupported reference type {:?}", String::from_utf8_lossy(&tag)))),
        })
    }
    fn value(&mut self, depth: usize) -> Result<Value> {
        self.node(depth)?;
        let tag: [u8; 4] = self.array()?;
        Ok(match &tag {
            b"obj " => {
                let count = self.count(MAX_NODES, 12)?;
                let mut values = Vec::new();
                for _ in 0..count {
                    values.push(self.reference(depth + 1)?);
                }
                Value::Reference(values)
            }
            b"Objc" => Value::Descriptor(self.descriptor(depth + 1)?),
            b"GlbO" => Value::GlobalObject(self.descriptor(depth + 1)?),
            b"VlLs" => {
                let count = self.count(MAX_NODES, 5)?;
                let mut values = Vec::new();
                for _ in 0..count {
                    values.push(self.value(depth + 1)?);
                }
                Value::List(values)
            }
            b"doub" => Value::Double(self.double()?),
            b"UntF" => Value::UnitFloat { unit: self.array()?, value: self.double()? },
            b"UnFl" => {
                let unit = self.array()?;
                let count = self.count(MAX_NODES, 8)?;
                let mut values = Vec::new();
                for _ in 0..count {
                    self.node(depth + 1)?;
                    values.push(self.double()?);
                }
                Value::UnitFloats { unit, values }
            }
            b"TEXT" => Value::Text(self.unicode()?),
            b"enum" => Value::Enumerated { type_id: self.id()?, value: self.id()? },
            b"long" => Value::Integer(self.i32()?),
            b"comp" => Value::LargeInteger(i64::from_be_bytes(self.array()?)),
            b"bool" => Value::Boolean(self.boolean()?),
            b"type" => Value::Class(self.class()?),
            b"GlbC" => Value::GlobalClass(self.class()?),
            b"alis" => Value::Alias(self.blob()?),
            b"Pth " => Value::Path(self.blob()?),
            b"tdta" => Value::RawData(self.blob()?),
            b"ObAr" => Value::ObjectArray { prefix: self.u32()?, body: self.descriptor(depth + 1)? },
            _ => return Err(self.error(&format!("unsupported descriptor type {:?}", String::from_utf8_lossy(&tag)))),
        })
    }
    fn step(&mut self) -> Result<Step> {
        self.node(0)?;
        let expanded = self.boolean()?;
        let enabled = self.boolean()?;
        let dialog = self.boolean()?;
        let dialog_options = self.byte()?;
        let tag: [u8; 4] = self.array()?;
        let event = match &tag {
            b"long" => Id::Code(self.array()?),
            b"TEXT" => {
                let n = self.count(MAX_STRING_UNITS, 1)?;
                if n == 0 {
                    return Err(self.error("empty string event identifier"));
                }
                Id::Str(self.take(n)?.to_vec())
            }
            _ => return Err(self.error("unsupported event identifier type")),
        };
        let n = self.count(MAX_STRING_UNITS, 1)?;
        let name_bytes = self.take(n)?.to_vec();
        let name = String::from_utf8_lossy(&name_bytes).trim_end_matches('\0').to_owned();
        let flag = self.i32()?;
        let start = self.pos;
        let descriptor = match flag {
            0 => None,
            -1 => Some(self.descriptor(0)?),
            _ => return Err(self.error("invalid descriptor-present flag")),
        };
        let descriptor_bytes = self.bytes.get(start..self.pos).ok_or_else(|| self.error("invalid descriptor range"))?.to_vec();
        Ok(Step { expanded, enabled, dialog, dialog_options, event, name, name_bytes, descriptor, descriptor_bytes })
    }
    fn action(&mut self) -> Result<Action> {
        self.node(0)?;
        let index = self.u16()?;
        let shift_key = self.boolean()?;
        let command_key = self.boolean()?;
        let color = self.u16()?;
        let name = self.unicode()?.to_string_lossy();
        let expanded = self.boolean()?;
        let count = self.count(100_000, 20)?;
        let mut steps = Vec::new();
        for _ in 0..count {
            steps.push(self.step()?);
        }
        Ok(Action { name, index, shift_key, command_key, color, expanded, steps })
    }
    fn set(&mut self) -> Result<ActionSet> {
        self.node(0)?;
        let name = self.unicode()?.to_string_lossy();
        let expanded = self.boolean()?;
        let count = self.count(4096, 15)?;
        let mut actions = Vec::new();
        for _ in 0..count {
            actions.push(self.action()?);
        }
        Ok(ActionSet { name, expanded, actions })
    }
}

/// Read all sets in a version-16 ATN file. Ordinary exported files have one set;
/// Photoshop's installed Essentials file has consecutive sets after one version.
/// Every input byte must form part of a complete supported record: trailing junk,
/// unknown tags, excessive nesting and truncated payloads are errors.
pub fn read_atn(bytes: &[u8], ctl: &prism_raster::Interrupt) -> Result<Vec<ActionSet>> {
    ctl.check().map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err("Photoshop ATN exceeds 64 MiB".into());
    }
    let mut r = Reader { bytes, pos: 0, nodes: 0, ctl };
    let version = r.u32()?;
    if version != 16 {
        return Err(r.error(&format!("unsupported version {version}; expected 16")));
    }
    let mut sets = Vec::new();
    loop {
        if sets.len() >= 256 {
            return Err(r.error("set count limit exceeded"));
        }
        sets.push(r.set()?);
        if r.pos == bytes.len() {
            break;
        }
    }
    ctl.check().map_err(|e| e.to_string())?;
    Ok(sets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_raster::Interrupt;

    fn u32b(n: u32) -> Vec<u8> {
        n.to_be_bytes().to_vec()
    }
    fn unicode(s: &str) -> Vec<u8> {
        let units: Vec<_> = s.encode_utf16().chain([0]).collect();
        [u32b(units.len() as u32), units.iter().flat_map(|n| n.to_be_bytes()).collect()].concat()
    }
    fn id(s: &str) -> Vec<u8> {
        [u32b(if s.len() == 4 { 0 } else { s.len() as u32 }), s.as_bytes().to_vec()].concat()
    }
    fn class(s: &str) -> Vec<u8> {
        [unicode(""), id(s)].concat()
    }
    fn descriptor(items: Vec<(&str, Vec<u8>)>) -> Vec<u8> {
        let mut b = [class("null"), u32b(items.len() as u32)].concat();
        for (key, value) in items {
            b.extend(id(key));
            b.extend(value);
        }
        b
    }
    fn value(tag: &[u8; 4], bytes: Vec<u8>) -> Vec<u8> {
        [tag.to_vec(), bytes].concat()
    }
    fn step(event: &str, data: Option<Vec<u8>>) -> Vec<u8> {
        let event = if event.len() == 4 {
            [b"long".to_vec(), event.as_bytes().to_vec()].concat()
        } else {
            [b"TEXT".to_vec(), u32b(event.len() as u32), event.as_bytes().to_vec()].concat()
        };
        [vec![1, 0, 1, 2], event, u32b(7), b"Example".to_vec(), u32b(if data.is_some() { u32::MAX } else { 0 }), data.unwrap_or_default()].concat()
    }
    fn set(steps: Vec<Vec<u8>>) -> Vec<u8> {
        let mut b = [unicode("Synthetic 한글"), vec![1], u32b(1), vec![0, 2, 1, 0, 0, 5], unicode("Action"), vec![1], u32b(steps.len() as u32)].concat();
        for step in steps {
            b.extend(step);
        }
        b
    }
    fn file(steps: Vec<Vec<u8>>) -> Vec<u8> {
        [u32b(16), set(steps)].concat()
    }

    #[test]
    fn names_flags_events_and_multiple_sets() {
        let mut b = file(vec![step("Invr", None), step("autoCutout", None)]);
        b.extend(set(vec![]));
        let sets = read_atn(&b, &Interrupt::NONE).unwrap();
        assert_eq!(sets.len(), 2);
        assert_eq!(sets[0].name, "Synthetic 한글");
        let action = &sets[0].actions[0];
        assert_eq!((action.index, action.color), (2, 5));
        assert!(action.shift_key && !action.command_key && action.expanded);
        let s = &action.steps[0];
        assert!(s.expanded && !s.enabled && s.dialog);
        assert_eq!(s.dialog_options, 2);
        assert!(matches!(s.event, Id::Code(_)) && s.event.is("Invr"));
        assert!(matches!(action.steps[1].event, Id::Str(_)));
        assert!(s.descriptor.is_none() && s.descriptor_bytes.is_empty());
    }

    #[test]
    fn descriptor_values_and_exact_source_bytes() {
        let nested = descriptor(vec![("text", value(b"TEXT", unicode("hello")))]);
        let d = descriptor(vec![
            ("bool", value(b"bool", vec![1])),
            ("long", value(b"long", (-3i32).to_be_bytes().to_vec())),
            ("comp", value(b"comp", i64::MIN.to_be_bytes().to_vec())),
            ("doub", value(b"doub", 3.25f64.to_be_bytes().to_vec())),
            ("unit", value(b"UntF", [b"#Pxl".to_vec(), 4.0f64.to_be_bytes().to_vec()].concat())),
            ("enum", value(b"enum", [id("Ordn"), id("Trgt")].concat())),
            ("list", value(b"VlLs", [u32b(2), value(b"TEXT", unicode("two")), value(b"Objc", nested.clone())].concat())),
            ("objt", value(b"GlbO", nested)),
            ("clas", value(b"type", class("Lyr "))),
            ("glob", value(b"GlbC", class("Dcmn"))),
            ("alia", value(b"alis", [u32b(3), vec![0, 128, 255]].concat())),
            ("path", value(b"Pth ", [u32b(2), vec![0, 255]].concat())),
            ("data", value(b"tdta", [u32b(1), vec![42]].concat())),
            (
                "obar",
                value(
                    b"ObAr",
                    [u32b(1), descriptor(vec![("unit", value(b"UnFl", [b"#Pxl".to_vec(), u32b(1), 9.0f64.to_be_bytes().to_vec()].concat()))])].concat(),
                ),
            ),
        ]);
        let parsed = read_atn(&file(vec![step("Mk  ", Some(d.clone()))]), &Interrupt::NONE).unwrap();
        let s = &parsed[0].actions[0].steps[0];
        assert_eq!(s.descriptor_bytes, d);
        let desc = s.descriptor.as_ref().unwrap();
        assert_eq!(desc.items.len(), 14);
        assert_eq!(desc.get("long"), Some(&Value::Integer(-3)));
        assert_eq!(desc.get("comp"), Some(&Value::LargeInteger(i64::MIN)));
        assert_eq!(desc.get("unit"), Some(&Value::UnitFloat { unit: *b"#Pxl", value: 4.0 }));
        assert_eq!(desc.get("alia"), Some(&Value::Alias(vec![0, 128, 255])));
        assert!(matches!(desc.get("obar"), Some(Value::ObjectArray { prefix: 1, .. })));
    }

    #[test]
    fn every_reference_form_keeps_its_class_including_index_and_identifier() {
        let mut refs = u32b(7);
        for (tag, tail) in [
            (b"prop", id("fsel")),
            (b"Clss", vec![]),
            (b"Enmr", [id("Ordn"), id("Trgt")].concat()),
            (b"rele", (-1i32).to_be_bytes().to_vec()),
            (b"Idnt", u32b(123)),
            (b"indx", u32b(2)),
            (b"name", unicode("Layer A")),
        ] {
            refs.extend(tag);
            refs.extend(class("Lyr "));
            refs.extend(tail);
        }
        let d = descriptor(vec![("null", value(b"obj ", refs))]);
        let parsed = read_atn(&file(vec![step("slct", Some(d))]), &Interrupt::NONE).unwrap();
        let Some(Value::Reference(items)) = parsed[0].actions[0].steps[0].descriptor.as_ref().unwrap().get("null") else { panic!() };
        assert_eq!(items.len(), 7);
        assert!(matches!(&items[4], ReferenceItem::Identifier { class, value: 123 } if class.class_id.is("Lyr ")));
        assert!(matches!(&items[5], ReferenceItem::Index { class, value: 2 } if class.class_id.is("Lyr ")));
    }

    #[test]
    fn truncation_and_trailing_junk_fail_without_panicking() {
        let b = file(vec![step("Invr", Some(descriptor(vec![("bool", value(b"bool", vec![1]))])))]);
        for end in 0..b.len() {
            assert!(read_atn(&b[..end], &Interrupt::NONE).is_err(), "end {end}");
        }
        let mut junk = b.clone();
        junk.push(0);
        assert!(read_atn(&junk, &Interrupt::NONE).is_err());
        for at in 0..b.len() {
            let mut corrupt = b.clone();
            corrupt[at] = 255;
            let _ = read_atn(&corrupt, &Interrupt::NONE);
        }
    }

    #[test]
    fn unknown_types_duplicate_keys_and_excessive_lengths_fail() {
        for items in [
            vec![("what", value(b"????", vec![0]))],
            vec![("bool", value(b"bool", vec![1])), ("bool", value(b"bool", vec![0]))],
            vec![("text", value(b"TEXT", u32b(u32::MAX)))],
        ] {
            assert!(read_atn(&file(vec![step("Invr", Some(descriptor(items)))]), &Interrupt::NONE).is_err());
        }
        assert!(read_atn(&[u32b(15), set(vec![])].concat(), &Interrupt::NONE).is_err());
        assert!(read_atn(&[u32b(16), u32b(u32::MAX)].concat(), &Interrupt::NONE).is_err());
        let mut invalid = step("Invr", None);
        invalid[1] = 2;
        assert!(read_atn(&file(vec![invalid]), &Interrupt::NONE).is_err());
    }

    #[test]
    fn depth_and_cancellation_are_bounded() {
        let mut v = value(b"bool", vec![1]);
        for _ in 0..40 {
            v = value(b"VlLs", [u32b(1), v].concat());
        }
        assert!(read_atn(&file(vec![step("Invr", Some(descriptor(vec![("list", v)])))]), &Interrupt::NONE).is_err());
        let ctl = Interrupt::cancel_only(&|| true);
        assert!(read_atn(&file(vec![]), &ctl).is_err());
    }

    proptest::proptest! {
        #[test]
        fn arbitrary_input_never_panics(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..8192)) {
            let _ = read_atn(&bytes, &Interrupt::NONE);
        }
    }

    #[test]
    #[ignore = "reads PRISM_TEST_ATN_FILES JSON array of local paths; never bundles proprietary actions"]
    fn local_installed_actions_parse_completely() {
        let paths: Vec<String> = serde_json::from_str(&std::env::var("PRISM_TEST_ATN_FILES").unwrap()).unwrap();
        assert!(!paths.is_empty());
        let mut total = (0, 0, 0);
        for path in paths {
            let bytes = std::fs::read(&path).unwrap();
            let sets = read_atn(&bytes, &Interrupt::NONE).unwrap_or_else(|e| panic!("{path}: {e}"));
            let actions: usize = sets.iter().map(|s| s.actions.len()).sum();
            let steps: usize = sets.iter().flat_map(|s| &s.actions).map(|a| a.steps.len()).sum();
            println!("{path}: {} sets, {actions} actions, {steps} steps", sets.len());
            total.0 += sets.len();
            total.1 += actions;
            total.2 += steps;
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        println!("total: {} sets, {} actions, {} steps", total.0, total.1, total.2);
    }
}
